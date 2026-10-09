//! Lexical package admission and unique witness inference, shared at boundaries.
use std::collections::HashMap;

use crate::hir::{HirEnum, HirStruct};
use crate::ids::DefId;
use crate::types::{ExistentialType, Type};

use super::kind::Kind;
use super::layout::Sizedness;
use super::normalize::{TypeNormalizationEnv, TypeNormalizer};
use super::visit::{visit_type_children, TypeVisitor};

/// Pair children only when every non-type component agrees canonically.
pub(crate) fn package_children<'a>(
    pattern: &'a ExistentialType,
    actual: &'a ExistentialType,
) -> Option<Vec<(&'a Type, &'a Type)>> {
    let left = pattern.children().collect::<Vec<_>>();
    let right = actual.children().collect::<Vec<_>>();
    if left.len() != right.len() {
        return None;
    }
    let mut children = right.iter();
    let rebuilt = pattern.map_types(|_| (**children.next().expect("equal child counts")).clone());
    if rebuilt != *actual {
        return None;
    }
    Some(left.into_iter().zip(right).collect())
}

pub(crate) fn normalize_payload_bindings(package: &ExistentialType) -> Result<Type, String> {
    struct Bindings<'a> {
        package: &'a ExistentialType,
        lexical: u32,
        objects: u32,
        nodes: usize,
        active: Vec<Type>,
    }
    impl super::visit::TryTypeFolder for Bindings<'_> {
        type Error = String;
        fn try_enter_binders(&mut self, _: &[Kind]) -> Result<(), String> {
            self.lexical = self
                .lexical
                .checked_add(1)
                .ok_or("package binder depth overflow")?;
            Ok(())
        }
        fn try_exit_binders(&mut self) -> Result<(), String> {
            self.lexical -= 1;
            Ok(())
        }
        fn try_enter_object(&mut self) -> Result<(), String> {
            self.objects = self
                .objects
                .checked_add(1)
                .ok_or("package object depth overflow")?;
            Ok(())
        }
        fn try_exit_object(&mut self) -> Result<(), String> {
            self.objects -= 1;
            Ok(())
        }
        fn try_fold_type(&mut self, ty: Type) -> Result<Type, String> {
            self.nodes += 1;
            if self.nodes > 100_000 || self.active.len() >= 128 {
                return Err("package associated-binding normalization limit exceeded".into());
            }
            let ty = super::visit::try_fold_type_children(ty, self)?;
            if let Type::Projection {
                ty: subject,
                trait_id,
                assoc_type,
                trait_args,
            } = &ty
            {
                if subject.as_ref()
                    == &(Type::BoundVar {
                        depth: self.lexical,
                        index: 0,
                        kind: self.package.binder.kind.clone(),
                    })
                {
                    for binding in &self.package.bounds.bindings {
                        if binding.key.member != assoc_type.assoc_type_id
                            || binding.key.trait_ref.trait_id != *trait_id
                            || assoc_type.owner != *trait_id
                        {
                            continue;
                        }
                        let args = binding
                            .key
                            .trait_ref
                            .type_args
                            .iter()
                            .map(|arg| {
                                super::substitution::shift_free_binders(
                                    arg,
                                    self.lexical,
                                    self.objects,
                                )
                                .map_err(|e| e.to_string())
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        if &args != trait_args {
                            continue;
                        }
                        if self.active.contains(&ty) {
                            return Err("cyclic package associated binding".into());
                        }
                        let replacement = super::substitution::shift_free_binders(
                            &binding.ty,
                            self.lexical,
                            self.objects,
                        )
                        .map_err(|e| e.to_string())?;
                        self.active.push(ty);
                        let result = self.try_fold_type(replacement);
                        self.active.pop();
                        return result;
                    }
                }
            }
            Ok(ty)
        }
    }
    super::visit::try_fold_type(
        package.payload.clone(),
        &mut Bindings {
            package,
            lexical: 0,
            objects: 0,
            nodes: 0,
            active: Vec::new(),
        },
    )
}

pub fn validate_lexical_binders(ty: &Type) -> Result<(), String> {
    struct Scope {
        groups: Vec<Vec<Kind>>,
        error: Option<String>,
    }
    impl TypeVisitor for Scope {
        fn enter_binders(&mut self, params: &[Kind]) {
            self.groups.push(params.to_vec());
        }
        fn exit_binders(&mut self) {
            self.groups.pop();
        }
        fn visit_type(&mut self, ty: &Type) {
            if let Type::BoundVar { depth, index, kind } = ty {
                if self
                    .groups
                    .iter()
                    .rev()
                    .nth(*depth as usize)
                    .and_then(|group| group.get(*index as usize))
                    != Some(kind)
                {
                    self.error = Some("out-of-scope or wrong-kind lexical type binder".into());
                }
            }
            visit_type_children(ty, self);
        }
    }
    let mut scope = Scope {
        groups: Vec::new(),
        error: None,
    };
    scope.visit_type(ty);
    scope.error.map_or(Ok(()), Err)
}

pub(crate) fn validate_package_layouts(
    ty: &Type,
    structs: &HashMap<DefId, HirStruct>,
    enums: &HashMap<DefId, HirEnum>,
    env: &TypeNormalizationEnv,
) -> Result<(), String> {
    validate_lexical_binders(ty)?;
    let mut error = None;
    crate::type_services::visit::visit_type(ty, &mut |ty: &Type| {
        if let Type::Exists(package) = ty {
            if package.binder.kind != Kind::Type || package.binder.maybe_unsized {
                error =
                    Some("existential packages currently require a Type-kind Sized witness".into());
            } else if !super::uniform_layout::is_uniform(
                &package.payload,
                structs,
                enums,
                env,
                &|ty| {
                    if matches!(
                        ty,
                        Type::BoundVar {
                            depth: 0,
                            index: 0,
                            kind: Kind::Type
                        }
                    ) {
                        Sizedness::Sized
                    } else {
                        Sizedness::Unknown
                    }
                },
            ) {
                error =
                    Some("existential payload does not have a structurally uniform layout".into());
            }
        }
    });
    error.map_or(Ok(()), Err)
}

pub fn infer_existential_witness(
    package: &ExistentialType,
    actual: &Type,
    env: &TypeNormalizationEnv,
) -> Result<Type, String> {
    infer_witness(package, actual, env, false)
}

/// Provisional inference does not authorize accepted HIR: the producer must
/// unify the instantiated payload invariantly and discharge every view later.
pub(crate) fn infer_pending_witness(
    package: &ExistentialType,
    actual: &Type,
    env: &TypeNormalizationEnv,
) -> Result<Type, String> {
    infer_witness(package, actual, env, true)
}

fn infer_witness(
    package: &ExistentialType,
    actual: &Type,
    env: &TypeNormalizationEnv,
    pending: bool,
) -> Result<Type, String> {
    struct Inference<'a> {
        value: Option<Type>,
        pending: bool,
        env: &'a TypeNormalizationEnv,
    }
    fn unresolved(ty: &Type) -> bool {
        crate::type_services::visit::type_any(ty, |ty| matches!(ty, Type::TypeVar(_)))
    }
    fn pair(
        pattern: &Type,
        actual: &Type,
        depth: u32,
        witness: &mut Inference,
    ) -> Result<(), String> {
        if matches!(pattern, Type::BoundVar { depth: d, index: 0, .. } if *d == depth) {
            validate_lexical_binders(actual)?;
            if super::substitution::has_free_object_self(actual) {
                return Err("package witness would capture a nested object receiver".into());
            }
            if crate::type_services::visit::type_any(actual, |ty| {
                matches!(ty, Type::Projection { .. })
                    || (matches!(ty, Type::Apply { .. })
                        && TypeNormalizer::new(witness.env)
                            .kind_of(ty)
                            .map_or(true, |kind| kind == Kind::Type))
            }) {
                return Err(
                    "package witness requires resolved projection/application descriptor evidence"
                        .into(),
                );
            }
            if crate::type_services::visit::type_any(actual, |ty| matches!(ty, Type::Error))
                || (!witness.pending && unresolved(actual))
            {
                return Err("package witness must be determined by the actual payload".into());
            }
            match &mut witness.value {
                Some(previous)
                    if witness.pending && (unresolved(previous) || unresolved(actual)) => {}
                Some(previous) if previous != actual => {
                    return Err("package payload contains different witnesses".into())
                }
                Some(_) => {}
                None => witness.value = Some(actual.clone()),
            }
            return Ok(());
        }
        let mismatch = || "package payload does not match its invariant template".to_string();
        match (pattern, actual) {
            (
                Type::Reference {
                    inner: p,
                    mutable: pm,
                },
                Type::Reference {
                    inner: a,
                    mutable: am,
                },
            ) if pm == am => pair(p, a, depth, witness),
            (Type::Pointer(p), Type::Pointer(a)) | (Type::Slice(p), Type::Slice(a)) => {
                pair(p, a, depth, witness)
            }
            (Type::Array(p, pn), Type::Array(a, an)) if pn == an => pair(p, a, depth, witness),
            (Type::Tuple(p), Type::Tuple(a)) if p.len() == a.len() => {
                for (p, a) in p.iter().zip(a) {
                    pair(p, a, depth, witness)?;
                }
                Ok(())
            }
            (Type::Struct { id: p, args: pa }, Type::Struct { id: a, args: aa })
            | (Type::Enum { id: p, args: pa }, Type::Enum { id: a, args: aa })
                if p == a && pa.len() == aa.len() =>
            {
                for (p, a) in pa.iter().zip(aa) {
                    pair(p, a, depth, witness)?;
                }
                Ok(())
            }
            (
                Type::Lambda {
                    params: p,
                    body: pb,
                },
                Type::Lambda {
                    params: a,
                    body: ab,
                },
            ) if p == a => pair(pb, ab, depth + 1, witness),
            (Type::Exists(p), Type::Exists(a)) if p.binder == a.binder => {
                pair_bounds(&p.bounds, &a.bounds, depth + 1, witness)?;
                pair(&p.payload, &a.payload, depth + 1, witness)
            }
            (Type::Object(p), Type::Object(a)) => pair_bounds(p, a, depth, witness),
            (
                Type::Function {
                    params: p,
                    ret: pr,
                    safety: ps,
                    callable_kind: pk,
                    captures: pc,
                    result_borrowing: pb,
                },
                Type::Function {
                    params: a,
                    ret: ar,
                    safety: as_,
                    callable_kind: ak,
                    captures: ac,
                    result_borrowing: ab,
                },
            ) if ps == as_ && pk == ak && pb == ab && p.len() == a.len() && pc.len() == ac.len() => {
                for (p, a) in p.iter().zip(a) {
                    pair(p, a, depth, witness)?;
                }
                pair(pr, ar, depth, witness)?;
                for (p, a) in pc.iter().zip(ac) {
                    if p.kind != a.kind {
                        return Err(mismatch());
                    }
                    pair(&p.ty, &a.ty, depth, witness)?;
                }
                Ok(())
            }
            (p, a) if p == a => Ok(()),
            _ => Err(mismatch()),
        }
    }
    fn pair_bounds(
        p: &crate::types::ObjectType,
        a: &crate::types::ObjectType,
        depth: u32,
        witness: &mut Inference,
    ) -> Result<(), String> {
        if p.principal.trait_id != a.principal.trait_id
            || p.guarantees.len() != a.guarantees.len()
            || p.bindings.len() != a.bindings.len()
        {
            return Err("package payload has different bound identities".into());
        }
        for (p, a) in std::iter::once(&p.principal)
            .chain(&p.guarantees)
            .chain(p.bindings.iter().map(|b| &b.key.trait_ref))
            .zip(
                std::iter::once(&a.principal)
                    .chain(&a.guarantees)
                    .chain(a.bindings.iter().map(|b| &b.key.trait_ref)),
            )
        {
            if p.trait_id != a.trait_id || p.type_args.len() != a.type_args.len() {
                return Err("package payload has different bound identities".into());
            }
            for (p, a) in p.type_args.iter().zip(&a.type_args) {
                pair(p, a, depth, witness)?;
            }
        }
        for (p, a) in p.bindings.iter().zip(&a.bindings) {
            if p.key.member != a.key.member {
                return Err("package payload has different associated members".into());
            }
            pair(&p.ty, &a.ty, depth, witness)?;
        }
        Ok(())
    }
    if package.binder.kind != Kind::Type || package.binder.maybe_unsized {
        return Err(
            "package witness requires implemented Type-kind Sized descriptor evidence".into(),
        );
    }
    let actual = TypeNormalizer::new(env)
        .normalize(actual)
        .map_err(|e| e.to_string())?;
    let pattern = TypeNormalizer::new(env)
        .normalize(&package.payload)
        .map_err(|e| e.to_string())?;
    let mut witness = Inference {
        value: None,
        pending,
        env,
    };
    pair(&pattern, &actual, 0, &mut witness)?;
    let witness = witness
        .value
        .ok_or("package witness is ambiguous; annotate or open the actual payload")?;
    if matches!(
        super::layout::TypeLayout::sizedness(&witness, &|_| Sizedness::Unknown),
        Sizedness::Unsized
    ) {
        return Err("a Sized existential cannot pack an unsized witness".into());
    }
    if TypeNormalizer::new(env)
        .kind_of(&witness)
        .map_err(|e| e.to_string())?
        != package.binder.kind
    {
        return Err("package witness has the wrong kind".into());
    }
    let instantiated = super::substitution::instantiate_existential(package, &witness)
        .map_err(|e| e.to_string())?;
    if !pending
        && TypeNormalizer::new(env)
            .normalize(&instantiated.payload)
            .map_err(|e| e.to_string())?
            != actual
    {
        return Err("package payload does not match its unique witness substitution".into());
    }
    Ok(witness)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{CrateId, HirLocalId, LocalDefId};
    use crate::types::{ExistentialBinder, ObjectType, TraitBound, WitnessId};

    fn hidden(depth: u32) -> Type {
        Type::BoundVar {
            depth,
            index: 0,
            kind: Kind::Type,
        }
    }
    fn reference(ty: Type) -> Type {
        Type::Reference {
            mutable: false,
            inner: Box::new(ty),
        }
    }
    fn package(payload: Type) -> ExistentialType {
        ExistentialType {
            binder: ExistentialBinder {
                kind: Kind::Type,
                maybe_unsized: false,
            },
            bounds: ObjectType::new(TraitBound {
                trait_id: DefId::new(CrateId(0), LocalDefId(1)),
                type_args: vec![],
            }),
            payload,
        }
    }
    fn witness(local: u32) -> Type {
        Type::Witness(WitnessId {
            owner: DefId::new(CrateId(0), LocalDefId(2)),
            local: HirLocalId(local),
        })
    }

    #[test]
    fn package_witness_is_unique_actual_and_invariant() {
        let package = package(Type::Tuple(vec![
            reference(hidden(0)),
            reference(hidden(0)),
        ]));
        let env = TypeNormalizationEnv::new();
        assert_eq!(
            infer_existential_witness(
                &package,
                &Type::Tuple(vec![reference(witness(3)), reference(witness(3))]),
                &env
            )
            .unwrap(),
            witness(3)
        );
        assert!(infer_existential_witness(
            &package,
            &Type::Tuple(vec![reference(witness(3)), reference(witness(4))]),
            &env
        )
        .is_err());
        assert!(infer_existential_witness(
            &package,
            &Type::Tuple(vec![reference(Type::I64), reference(Type::I32)]),
            &env
        )
        .is_err());
        assert!(infer_existential_witness(&self::package(Type::Unit), &Type::Unit, &env).is_err());
    }

    #[test]
    fn package_instantiation_does_not_capture_nested_existentials() {
        let inner = package(Type::Tuple(vec![
            reference(hidden(1)),
            reference(hidden(0)),
        ]));
        let outer = package(Type::Exists(Box::new(inner)));
        let instantiated =
            super::super::substitution::instantiate_existential(&outer, &witness(3)).unwrap();
        let Type::Exists(inner) = instantiated.payload else {
            panic!("nested")
        };
        assert_eq!(
            inner.payload,
            Type::Tuple(vec![reference(witness(3)), reference(hidden(0))])
        );
        assert!(validate_lexical_binders(&Type::Exists(Box::new(outer))).is_ok());
    }

    #[test]
    fn package_schema_round_trip_preserves_outer_lexical_binders() {
        let mut package = package(reference(hidden(0)));
        package.bounds.principal.type_args = vec![hidden(0), hidden(1)];
        let schema = super::super::substitution::existential_object_schema(&package).unwrap();
        assert_eq!(
            schema.principal.type_args,
            vec![Type::ObjectSelf { depth: 0 }, hidden(0)]
        );
        assert_eq!(
            super::super::substitution::existential_bounds_from_object_schema(&schema, &Kind::Type)
                .unwrap(),
            package.bounds
        );
    }

    #[test]
    fn lexical_binder_validation_checks_index_kind_and_uniform_storage() {
        let generic = Type::Generic(crate::types::GenericParamId {
            owner: DefId::new(CrateId(0), LocalDefId(9)),
            index: 0,
        });
        let pattern = Type::Exists(Box::new(package(reference(generic))));
        let actual = Type::Exists(Box::new(package(reference(hidden(0)))));
        let mut substitution = HashMap::new();
        assert!(!crate::selection::type_pattern_matches(
            &pattern,
            &actual,
            &mut substitution
        ));
        assert!(
            substitution.is_empty(),
            "an enclosing generic must not capture the package binder"
        );
        let mut package = package(reference(hidden(0)));
        let uniform = |ty: &Type| {
            validate_package_layouts(
                ty,
                &HashMap::new(),
                &HashMap::new(),
                &TypeNormalizationEnv::new(),
            )
        };
        assert!(uniform(&Type::Exists(Box::new(package.clone()))).is_ok());
        package.payload = hidden(0);
        assert!(uniform(&Type::Exists(Box::new(package.clone()))).is_err());
        package.payload = reference(Type::BoundVar {
            depth: 0,
            index: 1,
            kind: Kind::Type,
        });
        assert!(uniform(&Type::Exists(Box::new(package.clone()))).is_err());
        package.payload = reference(Type::BoundVar {
            depth: 0,
            index: 0,
            kind: Kind::arrow(Kind::Type, Kind::Type),
        });
        assert!(uniform(&Type::Exists(Box::new(package))).is_err());
    }
}
