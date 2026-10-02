//! Directional borrowed-object evidence, shared by production and HIR validation.
use std::collections::HashMap;

use crate::hir::*;
use crate::ids::DefId;
use crate::selection::{
    impl_receiver_pattern_substitution, type_pattern_matches, SelectionService,
};
use crate::type_services::substitution::instantiate_object_self;
use crate::types::{TraitBound, Type};

pub(crate) struct ObjectEvidenceContext<'a> {
    pub traits: &'a HashMap<DefId, HirTrait>,
    pub impls: &'a HashMap<DefId, HirImpl>,
    pub structs: &'a HashMap<DefId, HirStruct>,
    pub enums: &'a HashMap<DefId, HirEnum>,
    pub language_items: &'a HirLanguageItems,
    pub bounds: &'a HirGenericBounds,
}

impl ObjectEvidenceContext<'_> {
    /// Reclose a sized witness using only its dominating schema/dictionaries.
    /// This never searches for a concrete implementation of the hidden type.
    pub(crate) fn prove_opened(
        &self,
        source: &Type,
        target: &Type,
        witness: crate::types::WitnessId,
        available: &crate::types::ObjectType,
    ) -> Result<HirObjectEvidence, String> {
        let target_object =
            match (source, target) {
                (
                    Type::Reference {
                        mutable: source_mut,
                        inner,
                    },
                    Type::Reference {
                        mutable: target_mut,
                        inner: target,
                    },
                ) if inner.as_ref() == &Type::Witness(witness) && (!*target_mut || *source_mut) => {
                    target.as_ref()
                }
                (Type::Pointer(inner), Type::Pointer(target))
                    if inner.as_ref() == &Type::Witness(witness) =>
                {
                    target.as_ref()
                }
                _ => return Err(
                    "closing an opened object must preserve its exact witness/access capability"
                        .into(),
                ),
            };
        let Type::Object(required) = target_object else {
            return Err("closing an opened witness requires an object destination".into());
        };
        let instantiate = |object: &crate::types::ObjectType| {
            object
                .try_map_types(|ty| instantiate_object_self(ty, &Type::Witness(witness)))
                .map_err(|error| error.to_string())
        };
        let required = instantiate(required)?;
        let available = instantiate(available)?;
        let closure = |object: &crate::types::ObjectType| {
            let roots = std::iter::once(object.principal.clone())
                .chain(object.guarantees.iter().cloned())
                .collect::<Vec<_>>();
            crate::traits::evidence::implied_trait_bounds(
                self.traits,
                &Type::Witness(witness),
                &roots,
            )
        };
        let required_views = closure(&required);
        let available_views = closure(&available);
        if !required_views
            .iter()
            .all(|bound| available_views.contains(bound))
            || !required
                .bindings
                .iter()
                .all(|binding| available.bindings.contains(binding))
        {
            return Err(
                "closing an opened witness requires all target guarantees and associated bindings"
                    .into(),
            );
        }
        Ok(HirObjectEvidence::Concrete {
            source: Type::Witness(witness),
            views: required_views
                .into_iter()
                .map(|trait_ref| HirObjectView {
                    trait_ref,
                    origin: HirObjectWitnessOrigin::Bound,
                })
                .collect(),
        })
    }

    pub fn prove(&self, source: &Type, target: &Type) -> Result<HirObjectEvidence, String> {
        self.prove_with_assumptions(source, target, None)
    }

    pub fn prove_with_assumptions(
        &self,
        source: &Type,
        target: &Type,
        inferred: Option<&[TraitBound]>,
    ) -> Result<HirObjectEvidence, String> {
        let (source, target, source_mut, target_mut) = match (source, target) {
            (
                Type::Reference {
                    mutable: source_mut,
                    inner: source,
                },
                Type::Reference {
                    mutable: target_mut,
                    inner: target,
                },
            ) => (source, target, *source_mut, *target_mut),
            (Type::Pointer(source), Type::Pointer(target)) => (source, target, true, true),
            _ => {
                return Err(
                    "object coercion requires matching reference or raw-pointer handles".into(),
                )
            }
        };
        if target_mut && !source_mut {
            return Err("object coercion cannot upgrade shared access to mutable access".into());
        }
        let Type::Object(object) = target.as_ref() else {
            return Err("object coercion requires an object destination".into());
        };
        let roots = std::iter::once(object.principal.clone())
            .chain(object.guarantees.iter().cloned())
            .collect::<Vec<_>>();
        let required = crate::traits::evidence::implied_trait_bounds(
            self.traits,
            &Type::ObjectSelf { depth: 0 },
            &roots,
        );
        if let Type::Object(available) = source.as_ref() {
            let roots = std::iter::once(available.principal.clone())
                .chain(available.guarantees.iter().cloned())
                .collect::<Vec<_>>();
            let views = crate::traits::evidence::implied_trait_bounds(
                self.traits,
                &Type::ObjectSelf { depth: 0 },
                &roots,
            );
            if !required.iter().all(|bound| views.contains(bound))
                || !object
                    .bindings
                    .iter()
                    .all(|binding| available.bindings.contains(binding))
            {
                return Err(
                    "object upcast requires all target guarantees and associated bindings".into(),
                );
            }
            return Ok(HirObjectEvidence::Upcast {
                source: source.as_ref().clone(),
                target: target.as_ref().clone(),
            });
        }
        let selection = SelectionService::new(
            self.traits,
            self.impls,
            self.language_items.sized.as_ref().map(|item| item.trait_id),
            None,
            self.bounds,
        );
        let mut views = Vec::new();
        for formal in required {
            let bound = TraitBound {
                trait_id: formal.trait_id,
                type_args: formal
                    .type_args
                    .iter()
                    .map(|ty| {
                        instantiate_object_self(ty, source).map_err(|error| error.to_string())
                    })
                    .collect::<Result<_, _>>()?,
            };
            let assumption = if let Some(roots) = inferred {
                crate::traits::evidence::implied_trait_bounds(self.traits, source, roots)
                    .contains(&bound)
            } else if let Type::Generic(parameter) = source.as_ref() {
                let roots = self.bounds.get(parameter).cloned().unwrap_or_default();
                crate::traits::evidence::implied_trait_bounds(self.traits, source, &roots)
                    .contains(&bound)
            } else {
                false
            };
            let origin = if assumption {
                HirObjectWitnessOrigin::Bound
            } else if let Some(origin) = super::solve::object_protocol_origin(
                source,
                &bound,
                self.impls,
                self.structs,
                self.enums,
                self.language_items,
            ) {
                origin
            } else {
                let implementation = selection.select_trait_impl_strict(source, bound.trait_id, &bound.type_args).map_err(|_| "object construction requires a proven implementation of every guaranteed trait".to_string())?;
                let mut substitution =
                    impl_receiver_pattern_substitution(implementation, source, self.impls.values())
                        .ok_or("object implementation receiver mismatch")?;
                for (pattern, actual) in implementation.trait_arg_types.iter().zip(&bound.type_args)
                {
                    if !type_pattern_matches(pattern, actual, &mut substitution) {
                        return Err("object implementation argument mismatch".into());
                    }
                }
                let mut substitution = substitution
                    .into_iter()
                    .map(|(param, ty)| HirTypeBinding { param, ty })
                    .collect::<Vec<_>>();
                substitution.sort_by_key(|binding| binding.param);
                HirObjectWitnessOrigin::Impl {
                    impl_id: implementation.id,
                    substitution,
                }
            };
            for binding in object
                .bindings
                .iter()
                .filter(|binding| binding.key.trait_ref == formal)
            {
                let expected = instantiate_object_self(&binding.ty, source)
                    .map_err(|error| error.to_string())?;
                let actual = match &origin {
                    HirObjectWitnessOrigin::Impl {
                        impl_id,
                        substitution,
                    } => {
                        let substitution = substitution
                            .iter()
                            .map(|binding| (binding.param, binding.ty.clone()))
                            .collect();
                        self.impls[impl_id]
                            .associated_types
                            .iter()
                            .find(|member| member.id == binding.key.member)
                            .ok_or("missing implementation associated binding")?
                            .ty
                            .substitute_generics(&substitution)
                    }
                    HirObjectWitnessOrigin::Callable { .. } => {
                        let output = self
                            .language_items
                            .fn_once
                            .as_ref()
                            .filter(|item| item.trait_id == bound.trait_id)
                            .map(|item| item.output_id)
                            .or_else(|| {
                                self.language_items
                                    .fn_mut
                                    .as_ref()
                                    .filter(|item| item.trait_id == bound.trait_id)
                                    .map(|item| item.output_id)
                            })
                            .or_else(|| {
                                self.language_items
                                    .fn_trait
                                    .as_ref()
                                    .filter(|item| item.trait_id == bound.trait_id)
                                    .map(|item| item.output_id)
                            });
                        if output != Some(binding.key.member) {
                            return Err(
                                "object callable binding is not the marker-owned output".into()
                            );
                        }
                        let Type::Function { ret, .. } = source.as_ref() else {
                            return Err("native callable proof requires a function".into());
                        };
                        ret.as_ref().clone()
                    }
                    _ => {
                        return Err("associated object binding requires projection evidence".into())
                    }
                };
                if actual != expected {
                    return Err(
                        "object associated binding disagrees with its implementation".into(),
                    );
                }
            }
            views.push(HirObjectView {
                trait_ref: bound,
                origin,
            });
        }
        Ok(HirObjectEvidence::Concrete {
            source: source.as_ref().clone(),
            views,
        })
    }
}
