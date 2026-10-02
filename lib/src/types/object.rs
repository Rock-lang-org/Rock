use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::ids::{AssocTypeId, DefId};

use super::{TraitBound, Type};

/// An associated member belongs to an instantiated trait, not just a trait ID.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ObjectAssociatedType<T = Type> {
    pub trait_ref: TraitBound<T>,
    pub member: AssocTypeId,
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::ids::{CrateId, LocalDefId, TypeVarId};
    use crate::type_context::TypeContext;
    use crate::type_services::layout::{RuntimeTypeError, Sizedness, TypeLayout};
    use crate::type_services::normalize::{NormalizeError, TypeNormalizationEnv, TypeNormalizer};
    use crate::types::GenericParamId;

    use super::*;

    fn id(index: u32) -> DefId {
        DefId::new(CrateId(0), LocalDefId(index))
    }

    fn bound(index: u32, args: Vec<Type>) -> TraitBound {
        TraitBound {
            trait_id: id(index),
            type_args: args,
        }
    }

    fn binding(arg: Type, ty: Type) -> ObjectBinding {
        ObjectBinding {
            key: ObjectAssociatedType {
                trait_ref: bound(1, vec![arg]),
                member: AssocTypeId(2),
            },
            ty,
        }
    }

    #[test]
    fn object_signature_identity_and_interning_ignore_qualifier_order() {
        let mut first = ObjectType::new(bound(1, vec![Type::I64]));
        first.guarantees.extend([
            bound(1, vec![Type::Bool]),
            bound(2, vec![Type::Bool]),
            bound(3, vec![]),
        ]);
        first.bindings.extend([
            binding(Type::I64, Type::Bool),
            binding(Type::Bool, Type::I64),
        ]);
        let mut second = ObjectType::new(first.principal.clone());
        second
            .guarantees
            .extend(first.guarantees.iter().rev().cloned());
        second.bindings.extend(first.bindings.iter().rev().cloned());
        let first = Type::Object(Box::new(first));
        let second = Type::Object(Box::new(second));
        assert_eq!(first, second);
        let mut context = TypeContext::new();
        let interned = context.intern_type(&first);
        assert_eq!(interned, context.intern_type(&second));
        assert!(context.type_id_tree_is_valid(interned));
        assert_eq!(context.type_for(interned), first);
        assert_eq!(context.id_for_type(&second), Some(interned));
    }

    #[test]
    fn object_substitution_retains_conflicts_in_instantiated_member_keys() {
        let param = GenericParamId {
            owner: id(10),
            index: 0,
        };
        let mut object = ObjectType::new(bound(1, vec![]));
        object.bindings.extend([
            binding(Type::Generic(param), Type::Bool),
            binding(Type::I64, Type::I64),
        ]);
        let substituted = Type::Object(Box::new(object))
            .substitute_generics(&HashMap::from([(param, Type::I64)]));
        let Type::Object(object) = &substituted else {
            panic!("object expected")
        };
        assert_eq!(object.bindings.len(), 2);
        assert!(matches!(
            TypeNormalizer::new(&TypeNormalizationEnv::new()).normalize(&substituted),
            Err(NormalizeError::ConflictingObjectBinding(_))
        ));
    }

    #[test]
    fn object_walkers_remap_nested_arguments_and_associated_owners() {
        let mut object = ObjectType::new(bound(
            1,
            vec![Type::Struct {
                id: id(10),
                args: vec![],
            }],
        ));
        object
            .guarantees
            .insert(bound(3, vec![Type::TypeVar(TypeVarId(4))]));
        object.bindings.insert(binding(
            Type::Bool,
            Type::Generic(GenericParamId {
                owner: id(12),
                index: 0,
            }),
        ));
        let mut ty = Type::Object(Box::new(object));
        assert!(crate::type_services::visit::type_any(&ty, |ty| matches!(
            ty,
            Type::TypeVar(TypeVarId(4))
        )));
        ty = ty.substitute(&HashMap::from([(TypeVarId(4), Type::I64)]));
        ty.remap_def_ids(&mut |definition| DefId::new(CrateId(9), definition.local));
        let Type::Object(object) = ty else {
            panic!("object expected")
        };
        assert_eq!(object.principal.trait_id.crate_id, CrateId(9));
        assert!(
            matches!(&object.principal.type_args[0], Type::Struct { id, .. } if id.crate_id == CrateId(9))
        );
        assert!(object.guarantees.iter().all(
            |bound| bound.trait_id.crate_id == CrateId(9) && bound.type_args == vec![Type::I64]
        ));
        let binding = object.bindings.first().unwrap();
        assert_eq!(binding.key.trait_ref.trait_id.crate_id, CrateId(9));
        assert!(matches!(binding.ty, Type::Generic(param) if param.owner.crate_id == CrateId(9)));
    }

    #[test]
    fn object_signature_does_not_supply_payload_layout_or_borrow_freedom() {
        let object = Type::Object(Box::new(ObjectType::new(bound(1, vec![]))));
        assert_eq!(
            TypeLayout::sizedness(&object, &|_| Sizedness::Unknown),
            Sizedness::Unsized
        );
        assert!(!object.is_copy());
        assert!(object.contains_reference());
        let reference = Type::Reference {
            mutable: false,
            inner: Box::new(object),
        };
        assert_eq!(
            TypeLayout::sizedness(&reference, &|_| Sizedness::Unknown),
            Sizedness::Sized
        );
        assert_eq!(
            TypeLayout::validate_runtime_type(&reference),
            Err(RuntimeTypeError::ObjectRequiresEvidence)
        );
    }

    #[test]
    fn object_occurs_check_visits_associated_bindings() {
        let mut engine = crate::infer::InferenceEngine::new();
        let variable = engine.fresh_type_var();
        let mut object = ObjectType::new(bound(1, vec![]));
        object.bindings.insert(binding(Type::I64, variable.clone()));
        assert!(engine
            .unify(&variable, &Type::Object(Box::new(object)))
            .is_err());
    }

    #[test]
    fn object_projection_uses_the_exact_instantiated_associated_member() {
        struct NoImplLookup;
        impl crate::type_services::projection::ProjectionProvider for NoImplLookup {
            fn find_projection_impl(
                &self,
                _: &Type,
                _: DefId,
                _: &[Type],
            ) -> Option<crate::type_services::projection::ProjectionImpl> {
                panic!("object bindings must not rediscover a concrete implementation")
            }
        }
        let mut object = ObjectType::new(bound(1, vec![Type::I64]));
        object.guarantees.insert(bound(1, vec![Type::Bool]));
        object.bindings.extend([
            binding(Type::I64, Type::Bool),
            binding(Type::Bool, Type::I64),
        ]);
        for (argument, expected) in [(Type::I64, Type::Bool), (Type::Bool, Type::I64)] {
            let projection = Type::Projection {
                ty: Box::new(Type::Object(Box::new(object.clone()))),
                trait_id: id(1),
                assoc_type: crate::types::AssociatedTypeKey {
                    owner: id(1),
                    assoc_type_id: AssocTypeId(2),
                },
                trait_args: vec![argument],
            };
            assert_eq!(
                crate::type_services::projection::ProjectionNormalizer::normalize(
                    &NoImplLookup,
                    &projection
                ),
                expected
            );
        }
    }
}

/// Structural object identity. Ordering is local semantic ordering, never a
/// vtable slot order or an artifact's producer-to-consumer ABI.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ObjectType<T: Ord = Type> {
    pub principal: TraitBound<T>,
    pub guarantees: BTreeSet<TraitBound<T>>,
    pub bindings: BTreeSet<ObjectBinding<T>>,
}

/// Keep contradictory bindings until validation, including when substitution
/// makes two formerly distinct instantiated keys equal.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ObjectBinding<T = Type> {
    pub key: ObjectAssociatedType<T>,
    pub ty: T,
}

impl<T: Ord> ObjectType<T> {
    pub fn conflicting_binding(&self) -> Option<&ObjectAssociatedType<T>> {
        let mut previous = None;
        for binding in &self.bindings {
            if previous == Some(&binding.key) {
                return Some(&binding.key);
            }
            previous = Some(&binding.key);
        }
        None
    }

    pub fn new(principal: TraitBound<T>) -> Self {
        Self {
            principal,
            guarantees: BTreeSet::new(),
            bindings: BTreeSet::new(),
        }
    }

    pub fn children(&self) -> impl Iterator<Item = &T> {
        self.principal
            .type_args
            .iter()
            .chain(self.guarantees.iter().flat_map(|bound| &bound.type_args))
            .chain(self.bindings.iter().flat_map(|binding| {
                binding
                    .key
                    .trait_ref
                    .type_args
                    .iter()
                    .chain(std::iter::once(&binding.ty))
            }))
    }

    pub fn try_map_types<U: Ord, E>(
        &self,
        mut map: impl FnMut(&T) -> Result<U, E>,
    ) -> Result<ObjectType<U>, E> {
        fn map_bound<T, U, E>(
            bound: &TraitBound<T>,
            map: &mut impl FnMut(&T) -> Result<U, E>,
        ) -> Result<TraitBound<U>, E> {
            Ok(TraitBound {
                trait_id: bound.trait_id,
                type_args: bound.type_args.iter().map(map).collect::<Result<_, _>>()?,
            })
        }
        Ok(ObjectType {
            principal: map_bound(&self.principal, &mut map)?,
            guarantees: self
                .guarantees
                .iter()
                .map(|bound| map_bound(bound, &mut map))
                .collect::<Result<_, _>>()?,
            bindings: self
                .bindings
                .iter()
                .map(|binding| {
                    Ok(ObjectBinding {
                        key: ObjectAssociatedType {
                            trait_ref: map_bound(&binding.key.trait_ref, &mut map)?,
                            member: binding.key.member,
                        },
                        ty: map(&binding.ty)?,
                    })
                })
                .collect::<Result<_, _>>()?,
        })
    }

    pub fn map_types<U: Ord>(&self, mut map: impl FnMut(&T) -> U) -> ObjectType<U> {
        match self.try_map_types::<U, std::convert::Infallible>(|ty| Ok(map(ty))) {
            Ok(object) => object,
            Err(never) => match never {},
        }
    }

    pub(crate) fn try_remap_trait_ids<E>(
        &mut self,
        remap: &mut impl FnMut(DefId) -> Result<DefId, E>,
    ) -> Result<(), E> {
        self.principal.trait_id = remap(self.principal.trait_id)?;
        self.guarantees = std::mem::take(&mut self.guarantees)
            .into_iter()
            .map(|mut bound| {
                bound.trait_id = remap(bound.trait_id)?;
                Ok(bound)
            })
            .collect::<Result<_, E>>()?;
        self.bindings = std::mem::take(&mut self.bindings)
            .into_iter()
            .map(|mut binding| {
                binding.key.trait_ref.trait_id = remap(binding.key.trait_ref.trait_id)?;
                Ok(binding)
            })
            .collect::<Result<_, E>>()?;
        Ok(())
    }
}
