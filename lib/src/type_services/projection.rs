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
            Type::Object(object) => Type::Object(Box::new(
                object.map_types(|child| Self::normalize(provider, child)),
            )),
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

                if let Type::Object(object) = &resolved_base {
                    let mut bindings = object.bindings.iter().filter(|binding| {
                        binding.key.trait_ref.trait_id == *trait_id
                            && binding.key.trait_ref.type_args == resolved_trait_args
                            && binding.key.member == assoc_type.assoc_type_id
                    });
                    if let (Some(binding), None) = (bindings.next(), bindings.next()) {
                        return Self::normalize(provider, &binding.ty);
                    }
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
            } => Type::Function {
                params: params
                    .iter()
                    .map(|arg| Self::normalize(provider, arg))
                    .collect(),
                ret: Box::new(Self::normalize(provider, ret)),
                safety: *safety,
                callable_kind: *callable_kind,
                captures: captures
                    .iter()
                    .map(|capture| crate::types::FunctionCapture {
                        kind: capture.kind,
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
mod tests {
    use super::*;

    use crate::ids::{CrateId, LocalDefId};
    use crate::types::AssociatedTypeKey;

    #[derive(Default)]
    struct TestProjectionProvider {
        impls: Vec<(DefId, ProjectionImpl)>,
    }

    impl ProjectionProvider for TestProjectionProvider {
        fn find_projection_impl(
            &self,
            _base_ty: &Type,
            trait_id: DefId,
            _trait_args: &[Type],
        ) -> Option<ProjectionImpl> {
            self.impls
                .iter()
                .find(|(imp_trait_id, _)| *imp_trait_id == trait_id)
                .map(|(_, imp)| imp.clone())
        }
    }

    fn def_id(index: u32) -> DefId {
        DefId::new(CrateId(0), LocalDefId(index))
    }

    fn assoc(owner: DefId, index: u32) -> AssociatedTypeKey {
        AssociatedTypeKey {
            owner,
            assoc_type_id: AssocTypeId(index),
        }
    }

    fn test_impl(id: DefId, trait_id: DefId, assoc_ty: Type) -> (DefId, ProjectionImpl) {
        (
            trait_id,
            ProjectionImpl {
                impl_id: id,
                receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                    id: def_id(10),
                    args: vec![Type::Generic(GenericParamId {
                        owner: id,
                        index: 0,
                    })],
                }),
                trait_arg_types: Vec::new(),
                associated_types: vec![ProjectionAssociatedType {
                    id: AssocTypeId(0),
                    name: "Target".to_string(),
                    ty: assoc_ty,
                }],
            },
        )
    }

    #[test]
    fn projection_normalizer_resolves_impl_associated_type() {
        let impl_id = def_id(30);
        let trait_id = def_id(20);
        let provider = TestProjectionProvider {
            impls: vec![test_impl(
                impl_id,
                trait_id,
                Type::Generic(GenericParamId {
                    owner: impl_id,
                    index: 0,
                }),
            )],
        };
        let projection = Type::Projection {
            ty: Box::new(Type::Struct {
                id: def_id(10),
                args: vec![Type::I64],
            }),
            trait_id,
            assoc_type: assoc(trait_id, 0),
            trait_args: Vec::new(),
        };

        assert_eq!(
            ProjectionNormalizer::normalize(&provider, &projection),
            Type::I64
        );
    }

    #[test]
    fn projection_impl_substitution_uses_receiver_and_trait_arg_patterns() {
        let impl_id = def_id(70);
        let trait_id = def_id(71);
        let receiver_generic = GenericParamId {
            owner: impl_id,
            index: 0,
        };
        let trait_generic = GenericParamId {
            owner: trait_id,
            index: 0,
        };
        let projection = ProjectionImpl {
            impl_id,
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: def_id(72),
                args: vec![Type::Generic(receiver_generic)],
            }),
            trait_arg_types: vec![Type::Generic(trait_generic)],
            associated_types: vec![ProjectionAssociatedType {
                id: AssocTypeId(0),
                name: "Output".to_string(),
                ty: Type::Tuple(vec![
                    Type::Generic(receiver_generic),
                    Type::Generic(trait_generic),
                ]),
            }],
        };

        let subst = projection
            .projection_substitution(
                &Type::Struct {
                    id: def_id(72),
                    args: vec![Type::Bool],
                },
                &[Type::I64],
            )
            .expect("matching receiver and trait arguments should produce a substitution");

        assert_eq!(subst.get(&receiver_generic), Some(&Type::Bool));
        assert_eq!(subst.get(&trait_generic), Some(&Type::I64));
    }

    #[test]
    fn projection_impl_substitution_rejects_mismatched_authority() {
        let impl_id = def_id(73);
        let generic = GenericParamId {
            owner: impl_id,
            index: 0,
        };
        let projection = ProjectionImpl {
            impl_id,
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: def_id(74),
                args: vec![Type::Generic(generic)],
            }),
            trait_arg_types: vec![Type::I64],
            associated_types: Vec::new(),
        };

        assert!(projection
            .projection_substitution(
                &Type::Struct {
                    id: def_id(75),
                    args: vec![Type::Bool],
                },
                &[Type::I64],
            )
            .is_none());
        assert!(projection
            .projection_substitution(
                &Type::Struct {
                    id: def_id(74),
                    args: vec![Type::Bool],
                },
                &[],
            )
            .is_none());
        assert!(projection
            .projection_substitution(
                &Type::Struct {
                    id: def_id(74),
                    args: vec![Type::Bool],
                },
                &[Type::Bool],
            )
            .is_none());
    }

    #[test]
    fn projection_normalizer_refuses_mismatched_provider_impl() {
        let impl_id = def_id(76);
        let trait_id = def_id(77);
        let provider = TestProjectionProvider {
            impls: vec![test_impl(impl_id, trait_id, Type::I64)],
        };
        let projection = Type::Projection {
            ty: Box::new(Type::Struct {
                id: def_id(78),
                args: vec![Type::Bool],
            }),
            trait_id,
            assoc_type: assoc(trait_id, 0),
            trait_args: Vec::new(),
        };

        assert_eq!(
            ProjectionNormalizer::normalize(&provider, &projection),
            projection
        );
    }

    #[test]
    fn projection_normalizer_preserves_unresolved_projection_identity() {
        let trait_id = def_id(50);
        let provider = TestProjectionProvider::default();
        let projection = Type::Projection {
            ty: Box::new(Type::Generic(GenericParamId {
                owner: def_id(60),
                index: 0,
            })),
            trait_id,
            assoc_type: assoc(trait_id, 1),
            trait_args: vec![Type::Bool],
        };

        assert_eq!(
            ProjectionNormalizer::normalize(&provider, &projection),
            projection
        );
    }

    #[test]
    fn projection_normalizer_reduces_constructor_valued_projection_in_application() {
        let impl_id = def_id(80);
        let trait_id = def_id(81);
        let constructor_id = def_id(82);
        let provider = TestProjectionProvider {
            impls: vec![test_impl(
                impl_id,
                trait_id,
                Type::Constructor {
                    id: constructor_id,
                    flavor: crate::types::NominalTypeKind::Struct,
                },
            )],
        };
        let projection = Type::Projection {
            ty: Box::new(Type::Struct {
                id: def_id(10),
                args: vec![Type::I64],
            }),
            trait_id,
            assoc_type: assoc(trait_id, 0),
            trait_args: Vec::new(),
        };

        assert_eq!(
            ProjectionNormalizer::normalize(
                &provider,
                &Type::Apply {
                    constructor: Box::new(projection),
                    args: vec![Type::Bool],
                },
            ),
            Type::Apply {
                constructor: Box::new(Type::Constructor {
                    id: constructor_id,
                    flavor: crate::types::NominalTypeKind::Struct,
                }),
                args: vec![Type::Bool],
            }
        );
    }
}
