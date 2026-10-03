//! Structural independence of payload layout from hidden witness identities.
use std::collections::{HashMap, HashSet};

use crate::hir::{HirEnum, HirStruct, HirVariantFields};
use crate::ids::DefId;
use crate::types::Type;

use super::layout::{Sizedness, TypeLayout};
use super::normalize::{TypeNormalizationEnv, TypeNormalizer};

/// Proves layout independence, not concrete layout or borrow independence.
/// The caller supplies sizedness evidence for its abstract binder parameters.
pub(crate) fn is_uniform(
    ty: &Type,
    structs: &HashMap<DefId, HirStruct>,
    enums: &HashMap<DefId, HirEnum>,
    env: &TypeNormalizationEnv,
    abstract_sizedness: &impl Fn(&Type) -> Sizedness,
) -> bool {
    struct Proof<'a, F> {
        structs: &'a HashMap<DefId, HirStruct>,
        enums: &'a HashMap<DefId, HirEnum>,
        env: &'a TypeNormalizationEnv,
        abstract_sizedness: &'a F,
        active: HashSet<Type>,
        nodes: usize,
        binders: Vec<super::kind::Kind>,
    }

    impl<F: Fn(&Type) -> Sizedness> Proof<'_, F> {
        fn handle(&mut self, inner: &Type) -> bool {
            if let Type::BoundVar { depth, index, kind } = inner {
                if (*depth as usize) < self.binders.len() {
                    return *index == 0
                        && *kind == super::kind::Kind::Type
                        && self.binders.iter().rev().nth(*depth as usize) == Some(kind);
                }
                return (self.abstract_sizedness)(&Type::BoundVar {
                    depth: *depth - self.binders.len() as u32,
                    index: *index,
                    kind: kind.clone(),
                }) == Sizedness::Sized;
            }
            // Known unsized shapes have a fixed metadata width; unknown
            // constructor outputs do not acquire a thin-pointer exemption.
            if let Type::Array(element, _) = inner {
                return self.handle(element)
                    && !matches!(
                        element.as_ref(),
                        Type::Slice(_) | Type::Str | Type::Object(_)
                    );
            }
            if let Type::Tuple(elements) = inner {
                return elements.iter().all(|element| {
                    self.handle(element)
                        && !matches!(element, Type::Slice(_) | Type::Str | Type::Object(_))
                });
            }
            if let Type::Struct { args, .. } | Type::Enum { args, .. } = inner {
                let declared = match inner {
                    Type::Struct { id, .. } => self
                        .structs
                        .get(id)
                        .map(|definition| definition.generic_params.len()),
                    Type::Enum { id, .. } => self
                        .enums
                        .get(id)
                        .map(|definition| definition.generic_params.len()),
                    _ => None,
                };
                if declared != Some(args.len()) {
                    return false;
                }
                // Nominal sizedness alone does not prove a metadata width for
                // a hidden, possibly unsized tail. Indirect fields do.
                return self.ty(inner)
                    || args.iter().all(|arg| {
                        TypeLayout::sizedness(arg, &|param| {
                            (self.abstract_sizedness)(&Type::Generic(param))
                        }) == Sizedness::Sized
                            || (self.abstract_sizedness)(arg) == Sizedness::Sized
                    });
            }
            matches!(inner, Type::Slice(_) | Type::Str | Type::Object(_))
                || TypeLayout::sizedness(inner, &|param| {
                    (self.abstract_sizedness)(&Type::Generic(param))
                }) == Sizedness::Sized
                || (self.abstract_sizedness)(inner) == Sizedness::Sized
        }

        fn ty(&mut self, ty: &Type) -> bool {
            if self.active.len() >= 128 || self.nodes >= 100_000 {
                return false;
            }
            self.nodes += 1;
            let Ok(ty) = TypeNormalizer::new(self.env).normalize(ty) else {
                return false;
            };
            if !self.active.insert(ty.clone()) {
                return false;
            }
            let uniform = match &ty {
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
                | Type::Char
                | Type::Unit
                | Type::Never => true,
                Type::Reference { inner, .. } | Type::Pointer(inner) => self.handle(inner),
                Type::Array(inner, _) => self.ty(inner),
                Type::Tuple(elements) => elements.iter().all(|element| self.ty(element)),
                Type::Struct { id, args } => self.structs.get(id).is_some_and(|definition| {
                    if definition.generic_params.len() != args.len() {
                        return false;
                    }
                    let substitution = definition
                        .generic_params
                        .iter()
                        .zip(args)
                        .map(|(parameter, ty)| (parameter.id, ty.clone()))
                        .collect();
                    definition
                        .fields
                        .iter()
                        .all(|field| self.ty(&field.ty.substitute_generics(&substitution)))
                }),
                Type::Enum { id, args } => self.enums.get(id).is_some_and(|definition| {
                    if definition.generic_params.len() != args.len() {
                        return false;
                    }
                    let substitution = definition
                        .generic_params
                        .iter()
                        .zip(args)
                        .map(|(parameter, ty)| (parameter.id, ty.clone()))
                        .collect();
                    definition
                        .variants
                        .iter()
                        .all(|variant| match &variant.fields {
                            HirVariantFields::Unit => true,
                            HirVariantFields::Positional(fields) => fields
                                .iter()
                                .all(|field| self.ty(&field.substitute_generics(&substitution))),
                            HirVariantFields::Named(fields) => fields
                                .iter()
                                .all(|field| self.ty(&field.ty.substitute_generics(&substitution))),
                        })
                }),
                Type::Function { captures, .. } => {
                    captures.iter().all(|capture| self.ty(&capture.ty))
                }
                Type::Exists(package) => {
                    if package.binder.kind != super::kind::Kind::Type
                        || package.binder.maybe_unsized
                    {
                        false
                    } else {
                        self.binders.push(package.binder.kind.clone());
                        let uniform = self.ty(&package.payload);
                        self.binders.pop();
                        uniform
                    }
                }
                Type::Witness(_)
                | Type::ObjectSelf { .. }
                | Type::Generic(_)
                | Type::BoundVar { .. }
                | Type::Object(_)
                | Type::Slice(_)
                | Type::Str
                | Type::Constructor { .. }
                | Type::Apply { .. }
                | Type::Lambda { .. }
                | Type::Projection { .. }
                | Type::TypeVar(_)
                | Type::Error => false,
            };
            self.active.remove(&ty);
            uniform
        }
    }

    Proof {
        structs,
        enums,
        env,
        abstract_sizedness,
        active: HashSet::new(),
        nodes: 0,
        binders: Vec::new(),
    }
    .ty(ty)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{CrateId, HirLocalId, LocalDefId};
    use crate::type_services::kind::Kind;
    use crate::types::WitnessId;

    #[test]
    fn handles_are_uniform_but_direct_hidden_values_are_not() {
        let witness = Type::Witness(WitnessId {
            owner: DefId::new(CrateId(0), LocalDefId(1)),
            local: HirLocalId(2),
        });
        let uniform = |ty: &Type| {
            is_uniform(
                ty,
                &HashMap::new(),
                &HashMap::new(),
                &TypeNormalizationEnv::new(),
                &|_| Sizedness::Unknown,
            )
        };
        assert!(!uniform(&witness));
        assert!(!uniform(&Type::Tuple(vec![Type::I64, witness.clone()])));
        assert!(!uniform(&Type::Array(Box::new(witness.clone()), 0)));
        assert!(uniform(&Type::Pointer(Box::new(witness.clone()))));
        assert!(uniform(&Type::Reference {
            mutable: true,
            inner: Box::new(witness)
        }));
    }

    #[test]
    fn constructor_dependent_metadata_requires_sizedness_evidence() {
        let output = Type::Apply {
            constructor: Box::new(Type::BoundVar {
                depth: 0,
                index: 0,
                kind: Kind::arrow(Kind::Type, Kind::Type),
            }),
            args: vec![Type::I64],
        };
        let pointer = Type::Pointer(Box::new(output.clone()));
        let env = TypeNormalizationEnv::new();
        assert!(!is_uniform(
            &pointer,
            &HashMap::new(),
            &HashMap::new(),
            &env,
            &|_| Sizedness::Unknown
        ));
        assert!(is_uniform(
            &pointer,
            &HashMap::new(),
            &HashMap::new(),
            &env,
            &|ty| if ty == &output {
                Sizedness::Sized
            } else {
                Sizedness::Unknown
            }
        ));
    }

    #[test]
    fn nominal_payload_uniformity_comes_from_fields_not_owner_names() {
        let path = std::env::temp_dir().join("rock_uniform_layout/main.rk");
        let analysis = crate::analyze(&crate::Config {
            entry_file: path.clone(),
            no_std: true,
            no_prelude: true,
            source_providers: vec![crate::SourceProvider::Virtual {
                path,
                text: r#"
struct Indirect T
    < pointer: *T
struct Inline T
    < payload: T
struct Constant T
    < count: I64
main = !-> ()
"#
                .into(),
            }],
            ..Default::default()
        })
        .unwrap();
        let program = &analysis.hir.program;
        let env = &analysis.hir.normalization_env;
        let witness = Type::Witness(WitnessId {
            owner: DefId::new(CrateId(0), LocalDefId(1)),
            local: HirLocalId(2),
        });
        for (name, expected) in [("Indirect", true), ("Inline", false), ("Constant", true)] {
            let definition = program
                .structs
                .values()
                .find(|definition| definition.name == name)
                .unwrap();
            let payload = Type::Struct {
                id: definition.id,
                args: vec![witness.clone()],
            };
            assert_eq!(
                is_uniform(&payload, &program.structs, &program.enums, env, &|_| {
                    Sizedness::Unknown
                }),
                expected,
                "{name}"
            );
        }
    }
}
