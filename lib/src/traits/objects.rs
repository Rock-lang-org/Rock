//! Declaration-aware admission of existential object signatures.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use crate::hir::{HirAssociatedTypeDecl, HirPhase, HirTraitFor};
use crate::ids::{AssocTypeId, DefId};
use crate::type_services::kind::Kind;
use crate::type_services::normalize::{
    NormalizeError, TypeNormalizationEnv, TypeNormalizer, DEFAULT_MAX_NORMALIZATION_DEPTH,
    DEFAULT_MAX_NORMALIZATION_NODES,
};
use crate::type_services::substitution::{instantiate, shift_free_binders, SubstitutionError};
use crate::type_services::visit::{try_fold_type_children, TryTypeFolder};
use crate::types::{
    GenericParamDecl, GenericParamId, ObjectAssociatedType, ObjectType, Predicate, TraitBound, Type,
};

#[cfg(test)]
#[path = "objects_tests.rs"]
mod tests;

#[derive(Debug, Clone)]
pub struct TraitTypeHeader {
    pub generic_params: Vec<GenericParamDecl>,
    pub target: Option<GenericParamDecl>,
    pub associated_types: BTreeMap<AssocTypeId, Kind>,
    pub predicates: Vec<Predicate>,
}

impl TraitTypeHeader {
    pub fn new(
        params: &[GenericParamDecl],
        target: Option<&GenericParamDecl>,
        associated: &[HirAssociatedTypeDecl],
        predicates: &[Predicate],
    ) -> Self {
        Self {
            generic_params: params.to_vec(),
            target: target.cloned(),
            associated_types: associated
                .iter()
                .map(|member| (member.id, member.kind.clone()))
                .collect(),
            predicates: predicates.to_vec(),
        }
    }

    pub fn target_kind(&self) -> Kind {
        self.target
            .as_ref()
            .map(|target| target.kind.clone())
            .unwrap_or(Kind::Type)
    }

    pub fn try_remap_def_ids<E>(
        &mut self,
        remap: &mut impl FnMut(DefId) -> Result<DefId, E>,
    ) -> Result<(), E> {
        for parameter in &mut self.generic_params {
            parameter.id.owner = remap(parameter.id.owner)?;
        }
        if let Some(target) = &mut self.target {
            target.id.owner = remap(target.id.owner)?;
        }
        for predicate in &mut self.predicates {
            let Predicate::Trait {
                subject,
                trait_id,
                args,
            } = predicate;
            subject.try_remap_def_ids(remap)?;
            *trait_id = remap(*trait_id)?;
            for arg in args {
                arg.try_remap_def_ids(remap)?;
            }
        }
        Ok(())
    }

    fn self_param(&self, owner: DefId) -> GenericParamId {
        self.target
            .as_ref()
            .map(|target| target.id)
            .unwrap_or(GenericParamId {
                owner,
                index: self.generic_params.len() as u32,
            })
    }
}

pub trait ObjectTraitProvider {
    fn object_trait(&self, id: DefId) -> Option<TraitTypeHeader>;
}

impl<P: HirPhase> ObjectTraitProvider for HashMap<DefId, HirTraitFor<P>> {
    fn object_trait(&self, id: DefId) -> Option<TraitTypeHeader> {
        self.get(&id)
            .filter(|definition| definition.id == id)
            .map(|definition| {
                TraitTypeHeader::new(
                    &definition.generic_params,
                    definition.target.as_ref(),
                    &definition.associated_types,
                    &definition.predicates,
                )
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectAdmissionError {
    UnknownTrait(DefId),
    InvalidHeader(DefId),
    TargetKind {
        trait_id: DefId,
        kind: Kind,
    },
    Arity {
        trait_id: DefId,
        expected: usize,
        actual: usize,
    },
    Kind {
        trait_id: DefId,
        expected: Kind,
        actual: Kind,
    },
    BindingKind {
        key: ObjectAssociatedType,
        expected: Kind,
        actual: Kind,
    },
    MissingBinding(ObjectAssociatedType),
    UnknownMember(ObjectAssociatedType),
    UnrelatedBinding(ObjectAssociatedType),
    ConflictingBinding(ObjectAssociatedType),
    ProjectionCycle(ObjectAssociatedType),
    SupertraitCycle(Vec<DefId>),
    Normalization(NormalizeError),
    Substitution(SubstitutionError),
    ResourceLimit,
    UnboundObjectSelf,
}

impl std::fmt::Display for ObjectAdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownTrait(id) => write!(f, "unknown object trait {id:?}"),
            Self::InvalidHeader(id) => {
                write!(f, "invalid object trait parameter ownership for {id:?}")
            }
            Self::TargetKind { .. } => write!(f, "object trait requires a value-kind target"),
            Self::Arity {
                expected, actual, ..
            } => write!(
                f,
                "object trait argument arity mismatch: expected {expected}, found {actual}"
            ),
            Self::Kind {
                expected, actual, ..
            } => write!(
                f,
                "object trait argument kind mismatch: expected {expected}, found {actual}"
            ),
            Self::BindingKind {
                expected, actual, ..
            } => write!(
                f,
                "object associated binding kind mismatch: expected {expected}, found {actual}"
            ),
            Self::MissingBinding(key) => {
                write!(f, "missing required object associated binding {key:?}")
            }
            Self::UnknownMember(key) => write!(f, "unknown object associated member {key:?}"),
            Self::UnrelatedBinding(key) => write!(
                f,
                "object associated binding owner is outside the guaranteed trait closure: {key:?}"
            ),
            Self::ConflictingBinding(key) => {
                write!(f, "conflicting object associated binding {key:?}")
            }
            Self::ProjectionCycle(key) => write!(f, "object associated binding cycle at {key:?}"),
            Self::SupertraitCycle(path) => write!(f, "object supertrait cycle {path:?}"),
            Self::Normalization(error) => error.fmt(f),
            Self::Substitution(error) => write!(f, "object substitution failed: {error}"),
            Self::ResourceLimit => write!(f, "object admission resource limit exceeded"),
            Self::UnboundObjectSelf => write!(f, "object Self escapes its binding signature"),
        }
    }
}

impl From<NormalizeError> for ObjectAdmissionError {
    fn from(error: NormalizeError) -> Self {
        Self::Normalization(error)
    }
}
impl From<SubstitutionError> for ObjectAdmissionError {
    fn from(error: SubstitutionError) -> Self {
        Self::Substitution(error)
    }
}

struct ComponentNormalizer<'a> {
    object: &'a ObjectType,
    environment: &'a TypeNormalizationEnv,
    stack: Vec<ObjectAssociatedType>,
    lambdas: u32,
    objects: u32,
    depth: usize,
    nodes: usize,
}

impl TryTypeFolder for ComponentNormalizer<'_> {
    type Error = ObjectAdmissionError;

    fn try_enter_binders(&mut self, _: &[Kind]) -> Result<(), Self::Error> {
        self.lambdas += 1;
        Ok(())
    }
    fn try_exit_binders(&mut self) -> Result<(), Self::Error> {
        self.lambdas -= 1;
        Ok(())
    }
    fn try_enter_object(&mut self) -> Result<(), Self::Error> {
        self.objects += 1;
        Ok(())
    }
    fn try_exit_object(&mut self) -> Result<(), Self::Error> {
        self.objects -= 1;
        Ok(())
    }

    fn try_fold_type(&mut self, ty: Type) -> Result<Type, Self::Error> {
        self.nodes += 1;
        if self.nodes > DEFAULT_MAX_NORMALIZATION_NODES
            || self.depth >= DEFAULT_MAX_NORMALIZATION_DEPTH
        {
            return Err(ObjectAdmissionError::ResourceLimit);
        }
        self.depth += 1;
        let ty = try_fold_type_children(ty, self)?;
        let ty = TypeNormalizer::new(self.environment).normalize(&ty)?;
        let result = if let Type::Projection {
            ty: base,
            trait_id,
            assoc_type,
            trait_args,
        } = &ty
        {
            if matches!(base.as_ref(), Type::ObjectSelf { depth } if *depth == self.objects)
                && assoc_type.owner == *trait_id
            {
                let mut selected = None;
                for binding in &self.object.bindings {
                    if binding.key.trait_ref.trait_id != *trait_id
                        || binding.key.member != assoc_type.assoc_type_id
                    {
                        continue;
                    }
                    let arguments = binding
                        .key
                        .trait_ref
                        .type_args
                        .iter()
                        .map(|arg| shift_free_binders(arg, self.lambdas, self.objects))
                        .collect::<Result<Vec<_>, _>>()?;
                    if arguments == *trait_args {
                        if selected.is_some() {
                            return Err(ObjectAdmissionError::ConflictingBinding(
                                binding.key.clone(),
                            ));
                        }
                        selected = Some(binding.clone());
                    }
                }
                let key = ObjectAssociatedType {
                    trait_ref: TraitBound {
                        trait_id: *trait_id,
                        type_args: trait_args.clone(),
                    },
                    member: assoc_type.assoc_type_id,
                };
                let binding = selected.ok_or(ObjectAdmissionError::MissingBinding(key))?;
                if self.stack.contains(&binding.key) {
                    return Err(ObjectAdmissionError::ProjectionCycle(binding.key));
                }
                self.stack.push(binding.key);
                let value = shift_free_binders(&binding.ty, self.lambdas, self.objects)?;
                let value = self.try_fold_type(value)?;
                self.stack.pop();
                value
            } else {
                ty
            }
        } else {
            ty
        };
        self.depth -= 1;
        Ok(result)
    }
}

fn normalize_component(
    ty: &Type,
    object: &ObjectType,
    environment: &TypeNormalizationEnv,
) -> Result<Type, ObjectAdmissionError> {
    ComponentNormalizer {
        object,
        environment,
        stack: Vec::new(),
        lambdas: 0,
        objects: 0,
        depth: 0,
        nodes: 0,
    }
    .try_fold_type(ty.clone())
}

fn normalize_bound(
    bound: &TraitBound,
    object: &ObjectType,
    environment: &TypeNormalizationEnv,
) -> Result<TraitBound, ObjectAdmissionError> {
    Ok(TraitBound {
        trait_id: bound.trait_id,
        type_args: bound
            .type_args
            .iter()
            .map(|arg| normalize_component(arg, object, environment))
            .collect::<Result<_, _>>()?,
    })
}

fn header(
    provider: &impl ObjectTraitProvider,
    bound: &TraitBound,
    environment: &TypeNormalizationEnv,
) -> Result<TraitTypeHeader, ObjectAdmissionError> {
    let declaration = provider
        .object_trait(bound.trait_id)
        .ok_or(ObjectAdmissionError::UnknownTrait(bound.trait_id))?;
    if declaration.target_kind() != Kind::Type {
        return Err(ObjectAdmissionError::TargetKind {
            trait_id: bound.trait_id,
            kind: declaration.target_kind(),
        });
    }
    if declaration.generic_params.len() != bound.type_args.len() {
        return Err(ObjectAdmissionError::Arity {
            trait_id: bound.trait_id,
            expected: declaration.generic_params.len(),
            actual: bound.type_args.len(),
        });
    }
    if declaration
        .generic_params
        .iter()
        .any(|param| param.id.owner != bound.trait_id)
        || declaration.self_param(bound.trait_id).owner != bound.trait_id
    {
        return Err(ObjectAdmissionError::InvalidHeader(bound.trait_id));
    }
    let parameters: std::collections::HashSet<_> = declaration
        .generic_params
        .iter()
        .map(|param| param.id)
        .collect();
    if parameters.len() != declaration.generic_params.len()
        || parameters.contains(&declaration.self_param(bound.trait_id))
    {
        return Err(ObjectAdmissionError::InvalidHeader(bound.trait_id));
    }
    for predicate in &declaration.predicates {
        let Predicate::Trait { subject, args, .. } = predicate;
        if std::iter::once(subject)
            .chain(args)
            .any(crate::type_services::substitution::has_free_object_self)
        {
            return Err(ObjectAdmissionError::InvalidHeader(bound.trait_id));
        }
    }
    for (parameter, argument) in declaration.generic_params.iter().zip(&bound.type_args) {
        let actual = TypeNormalizer::new(environment).kind_of(argument)?;
        if actual != parameter.kind {
            return Err(ObjectAdmissionError::Kind {
                trait_id: bound.trait_id,
                expected: parameter.kind.clone(),
                actual,
            });
        }
    }
    Ok(declaration)
}

/// The result contains the complete normalized guaranteed closure. Its hidden
/// receiver is ObjectSelf(0), not an unsized object or a fabricated definition.
pub fn admit_object(
    object: &ObjectType,
    provider: &impl ObjectTraitProvider,
    environment: &TypeNormalizationEnv,
) -> Result<ObjectType, ObjectAdmissionError> {
    admit_object_at_depth(object, provider, environment, 0)
}

fn admit_object_at_depth(
    object: &ObjectType,
    provider: &impl ObjectTraitProvider,
    environment: &TypeNormalizationEnv,
    outer_objects: u32,
) -> Result<ObjectType, ObjectAdmissionError> {
    if object
        .guarantees
        .len()
        .saturating_add(object.bindings.len())
        >= DEFAULT_MAX_NORMALIZATION_NODES
    {
        return Err(ObjectAdmissionError::ResourceLimit);
    }
    if crate::type_services::substitution::has_free_object_self_at_depth(
        &Type::Object(Box::new(object.clone())),
        outer_objects,
    ) {
        return Err(ObjectAdmissionError::UnboundObjectSelf);
    }
    let normalized =
        TypeNormalizer::new(environment).normalize(&Type::Object(Box::new(object.clone())))?;
    let Type::Object(mut object) = normalized else {
        unreachable!("object normalization preserves its constructor")
    };
    let snapshot = object.clone();
    object = Box::new(object.try_map_types(|ty| normalize_component(ty, &snapshot, environment))?);
    if let Some(key) = object.conflicting_binding() {
        return Err(ObjectAdmissionError::ConflictingBinding(key.clone()));
    }

    let roots = std::iter::once(object.principal.clone()).chain(object.guarantees.iter().cloned());
    let mut pending: VecDeque<_> = roots.map(|bound| (bound, Vec::new())).collect();
    let mut views = BTreeSet::new();
    while let Some((bound, mut path)) = pending.pop_front() {
        if views.len() >= DEFAULT_MAX_NORMALIZATION_NODES
            || path.len() >= DEFAULT_MAX_NORMALIZATION_DEPTH
        {
            return Err(ObjectAdmissionError::ResourceLimit);
        }
        let bound = normalize_bound(&bound, &object, environment)?;
        if path.contains(&bound.trait_id) {
            path.push(bound.trait_id);
            return Err(ObjectAdmissionError::SupertraitCycle(path));
        }
        if !views.insert(bound.clone()) {
            continue;
        }
        let declaration = header(provider, &bound, environment)?;
        for (member, expected) in &declaration.associated_types {
            let key = ObjectAssociatedType {
                trait_ref: bound.clone(),
                member: *member,
            };
            let binding = object
                .bindings
                .iter()
                .find(|binding| binding.key == key)
                .ok_or_else(|| ObjectAdmissionError::MissingBinding(key.clone()))?;
            let actual = TypeNormalizer::new(environment).kind_of(&binding.ty)?;
            if &actual != expected {
                return Err(ObjectAdmissionError::BindingKind {
                    key,
                    expected: expected.clone(),
                    actual,
                });
            }
        }
        let mut arguments: HashMap<_, _> = declaration
            .generic_params
            .iter()
            .zip(&bound.type_args)
            .map(|(param, arg)| (param.id, arg.clone()))
            .collect();
        arguments.insert(
            declaration.self_param(bound.trait_id),
            Type::ObjectSelf { depth: 0 },
        );
        path.push(bound.trait_id);
        for predicate in &declaration.predicates {
            let Predicate::Trait {
                subject,
                trait_id,
                args,
            } = predicate;
            let subject =
                normalize_component(&instantiate(subject, &arguments)?, &object, environment)?;
            if subject != (Type::ObjectSelf { depth: 0 }) {
                continue;
            }
            let args = args
                .iter()
                .map(|arg| instantiate(arg, &arguments))
                .collect::<Result<_, _>>()?;
            pending.push_back((
                TraitBound {
                    trait_id: *trait_id,
                    type_args: args,
                },
                path.clone(),
            ));
        }
    }
    for binding in &object.bindings {
        if !views.contains(&binding.key.trait_ref) {
            return Err(ObjectAdmissionError::UnrelatedBinding(binding.key.clone()));
        }
        let declaration = header(provider, &binding.key.trait_ref, environment)?;
        if !declaration
            .associated_types
            .contains_key(&binding.key.member)
        {
            return Err(ObjectAdmissionError::UnknownMember(binding.key.clone()));
        }
    }
    views.remove(&object.principal);
    object.guarantees = views;
    Ok(*object)
}

pub fn admit_type_objects(
    ty: &Type,
    provider: &impl ObjectTraitProvider,
    environment: &TypeNormalizationEnv,
) -> Result<Type, ObjectAdmissionError> {
    struct Admit<'a, P> {
        provider: &'a P,
        environment: &'a TypeNormalizationEnv,
        objects: u32,
    }
    impl<P: ObjectTraitProvider> TryTypeFolder for Admit<'_, P> {
        type Error = ObjectAdmissionError;
        fn try_enter_object(&mut self) -> Result<(), Self::Error> {
            self.objects += 1;
            Ok(())
        }
        fn try_exit_object(&mut self) -> Result<(), Self::Error> {
            self.objects -= 1;
            Ok(())
        }
        fn try_fold_type(&mut self, ty: Type) -> Result<Type, Self::Error> {
            if let Type::ObjectSelf { depth } = &ty {
                if *depth >= self.objects {
                    return Err(ObjectAdmissionError::UnboundObjectSelf);
                }
            }
            let ty = if let Type::Object(object) = ty {
                Type::Object(Box::new(admit_object_at_depth(
                    &object,
                    self.provider,
                    self.environment,
                    self.objects,
                )?))
            } else {
                ty
            };
            try_fold_type_children(ty, self)
        }
    }
    Admit {
        provider,
        environment,
        objects: 0,
    }
    .try_fold_type(ty.clone())
}
