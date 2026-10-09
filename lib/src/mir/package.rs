//! Allocation-free existential storage and exact payload descriptor authority.
use std::collections::{BTreeMap, HashSet};

use crate::ids::{DefId, TypeId};
use crate::type_context::TypeContext;
use crate::type_services::kind::Kind;
use crate::types::Type;

use super::{MirBackendContract, MirErasedError, MirErasedType, MirNominalLayout, MirVtableId};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MirUniformLayout {
    Native(TypeId),
    ThinHandle,
    Tuple(Vec<Self>),
    Array {
        element: Box<Self>,
        len: usize,
    },
    Struct {
        definition: DefId,
        fields: Vec<Self>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirPackageSchema {
    pub package: TypeId,
    pub object: TypeId,
    pub payload_layout: MirUniformLayout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirPackageWitnessEvidence {
    Concrete { witness: TypeId, table: MirVtableId },
    Scoped { scope: u32, view: TypeId },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirPackagePayloadDescriptor {
    Concrete(TypeId),
    UniformDropFree,
    FromPackage(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirPackagePackPlan {
    pub package: TypeId,
    pub witness: MirPackageWitnessEvidence,
    pub payload_descriptor: MirPackagePayloadDescriptor,
}

/// No representative substitution is used: a hidden sized pointee proves only
/// a thin handle, never its pointee layout or its owner's destructor.
pub fn uniform_payload_layout(
    contract: &MirBackendContract,
    tc: &mut TypeContext,
    ty: &Type,
) -> Result<MirUniformLayout, String> {
    fn hidden(ty: &Type) -> bool {
        crate::type_services::visit::type_any(ty, |ty| {
            matches!(
                ty,
                Type::BoundVar { .. } | Type::Witness(_) | Type::ObjectSelf { .. }
            )
        })
    }
    fn sized_pointee(ty: &Type) -> bool {
        match ty {
            Type::BoundVar {
                depth: 0,
                index: 0,
                kind: Kind::Type,
            }
            | Type::Witness(_) => true,
            Type::Tuple(fields) => fields.iter().all(sized_pointee),
            Type::Array(element, _) => sized_pointee(element),
            Type::Struct { .. } | Type::Enum { .. } => true,
            _ => crate::type_services::layout::TypeLayout::validate_runtime_type(ty).is_ok(),
        }
    }
    fn build(
        contract: &MirBackendContract,
        tc: &mut TypeContext,
        ty: &Type,
        active: &mut HashSet<Type>,
    ) -> Result<MirUniformLayout, String> {
        if !hidden(ty) {
            let id = super::intern_backend_type(tc, ty);
            return super::object_runtime_type_is_valid(contract, tc, id)
                .then_some(MirUniformLayout::Native(id))
                .ok_or_else(|| "package contains a non-runtime native layout".into());
        }
        Ok(match ty {
            Type::Reference { inner, .. } | Type::Pointer(inner) if sized_pointee(inner) => {
                MirUniformLayout::ThinHandle
            }
            Type::Tuple(fields) => MirUniformLayout::Tuple(
                fields
                    .iter()
                    .map(|field| build(contract, tc, field, active))
                    .collect::<Result<_, _>>()?,
            ),
            Type::Array(element, len) => MirUniformLayout::Array {
                element: Box::new(build(contract, tc, element, active)?),
                len: *len,
            },
            Type::Struct { id, args } => {
                if !active.insert(ty.clone()) {
                    return Err("recursive inline package payload".into());
                }
                let Some(MirNominalLayout::Struct {
                    fields,
                    generic_params,
                    ..
                }) = contract.nominal_layouts.get(id)
                else {
                    return Err("package nominal layout is unavailable".into());
                };
                if generic_params.len() != args.len() {
                    return Err("package nominal argument arity mismatch".into());
                }
                let substitution = generic_params
                    .iter()
                    .copied()
                    .zip(args.iter().cloned())
                    .collect();
                let field_types = fields
                    .iter()
                    .map(|(_, field)| tc.type_for(*field).substitute_generics(&substitution))
                    .collect::<Vec<_>>();
                let fields = field_types
                    .iter()
                    .map(|field| build(contract, tc, field, active))
                    .collect::<Result<_, _>>()?;
                active.remove(ty);
                MirUniformLayout::Struct {
                    definition: *id,
                    fields,
                }
            }
            _ => return Err("existential payload has no proven uniform layout".into()),
        })
    }
    build(contract, tc, ty, &mut HashSet::new())
}

pub fn package_schema(
    contract: &MirBackendContract,
    tc: &mut TypeContext,
    package: TypeId,
) -> Result<MirPackageSchema, String> {
    if !tc.type_id_tree_is_valid(package) {
        return Err("invalid package type identity".into());
    }
    let Type::Exists(existential) = tc.type_for(package) else {
        return Err("package schema requires an existential type".into());
    };
    use crate::type_services::visit::TypeVisitor;
    #[derive(Default)]
    struct Closed {
        binders: Vec<Vec<Kind>>,
        objects: u32,
        invalid: bool,
    }
    impl TypeVisitor for Closed {
        fn enter_binders(&mut self, kinds: &[Kind]) {
            self.binders.push(kinds.to_vec());
        }
        fn exit_binders(&mut self) {
            self.binders.pop();
        }
        fn enter_object(&mut self) {
            self.objects += 1;
        }
        fn exit_object(&mut self) {
            self.objects -= 1;
        }
        fn visit_type(&mut self, ty: &Type) {
            self.invalid |= match ty {
                Type::BoundVar { depth, index, kind } => {
                    self.binders
                        .len()
                        .checked_sub(*depth as usize + 1)
                        .and_then(|binder| self.binders[binder].get(*index as usize))
                        != Some(kind)
                }
                Type::ObjectSelf { depth } => *depth >= self.objects,
                Type::Witness(_)
                | Type::Generic(_)
                | Type::TypeVar(_)
                | Type::Error
                | Type::Projection { .. } => true,
                _ => false,
            };
            crate::type_services::visit::visit_type_children(ty, self);
        }
    }
    let mut closed = Closed::default();
    closed.visit_type(&Type::Exists(existential.clone()));
    if closed.invalid {
        return Err("package has an invalid lexical binder or free type".into());
    }
    if !crate::type_services::facts::TypeFacts::is_codegen_concrete(&Type::Exists(
        existential.clone(),
    )) {
        return Err("package type contains a free or unresolved type".into());
    }
    if existential.binder.kind != Kind::Type || existential.binder.maybe_unsized {
        return Err("constructor-kind or unsized existential descriptors are not supported".into());
    }
    let object = crate::type_services::substitution::existential_object_schema(&existential)
        .map_err(|error| error.to_string())?;
    let object = tc
        .id_for_type(&Type::Object(Box::new(object)))
        .filter(|object| contract.object_schemas.contains_key(object))
        .ok_or("existential dispatch schema is unavailable")?;
    let payload_layout = uniform_payload_layout(contract, tc, &existential.payload)?;
    Ok(MirPackageSchema {
        package,
        object,
        payload_layout,
    })
}

pub fn package_schema_is_valid(
    contract: &MirBackendContract,
    tc: &TypeContext,
    package: TypeId,
) -> bool {
    contract.packages.get(&package).is_some_and(|schema| {
        package_schema(contract, &mut tc.clone(), package).is_ok_and(|expected| *schema == expected)
    })
}

pub fn package_payload_type(
    package: TypeId,
    witness: &Type,
    tc: &TypeContext,
) -> Result<Type, MirErasedError> {
    let Type::Exists(existential) = tc.type_for(package) else {
        return Err(MirErasedError::InvalidType);
    };
    crate::type_services::substitution::instantiate_existential(&existential, witness)
        .map(|instantiated| instantiated.payload)
        .map_err(|_| MirErasedError::InvalidType)
}

pub fn scoped_package_payload(
    package: TypeId,
    scope: u32,
    witness: crate::types::WitnessId,
    tc: &mut TypeContext,
) -> Result<MirErasedType, MirErasedError> {
    let payload = package_payload_type(package, &Type::Witness(witness), tc)?;
    MirErasedType::from_type_scoped(
        &payload,
        &BTreeMap::new(),
        None,
        &BTreeMap::from([(witness, scope)]),
        tc,
        &Default::default(),
    )
}

/// Owners with user Drop never qualify, even when their physical fields are pointers.
pub fn package_payload_drop_free(
    contract: &MirBackendContract,
    tc: &TypeContext,
    package: TypeId,
) -> bool {
    fn prove(
        contract: &MirBackendContract,
        tc: &TypeContext,
        ty: &Type,
        active: &mut HashSet<Type>,
    ) -> bool {
        match ty {
            Type::Reference { .. } | Type::Pointer(_) => true,
            Type::Tuple(fields) => fields
                .iter()
                .all(|field| prove(contract, tc, field, active)),
            Type::Array(element, _) => prove(contract, tc, element, active),
            Type::Struct { id, args } => {
                // A witness-dependent custom Drop cannot be justified by a currently
                // materialized concrete specialization, so reject all nominal owners.
                let _ = (id, args, active);
                false
            }
            _ => tc.id_for_type(ty).is_some_and(|id| {
                super::payload_drop_plan(contract, &mut tc.clone(), id)
                    .is_ok_and(|plan| !plan.has_drop())
            }),
        }
    }
    matches!(tc.type_for(package), Type::Exists(existential) if prove(contract, tc, &existential.payload, &mut HashSet::new()))
}

/// Only structurally drop-free aggregates get synthesized global descriptors.
/// Whole owner payloads use their original descriptor retained by PackageOpen.
pub fn erased_uniform_layout(
    contract: &MirBackendContract,
    tc: &mut TypeContext,
    ty: &MirErasedType,
) -> Result<MirUniformLayout, MirErasedError> {
    fn sized(ty: &MirErasedType, tc: &TypeContext) -> bool {
        match ty {
            MirErasedType::Scoped(_) => true,
            MirErasedType::Concrete(id) => {
                tc.type_id_tree_is_valid(*id)
                    && crate::type_services::layout::TypeLayout::sizedness(
                        &tc.type_for(*id),
                        &|_| crate::type_services::layout::Sizedness::Unknown,
                    ) == crate::type_services::layout::Sizedness::Sized
            }
            MirErasedType::Reference { .. }
            | MirErasedType::Pointer(_)
            | MirErasedType::Nominal { .. } => true,
            MirErasedType::Tuple(fields) => fields.iter().all(|field| sized(field, tc)),
            MirErasedType::Array { element, .. } => sized(element, tc),
            _ => false,
        }
    }
    Ok(match ty {
        MirErasedType::Concrete(id)
            if super::object_runtime_type_is_valid(contract, tc, *id)
                && super::payload_drop_plan(contract, tc, *id)
                    .is_ok_and(|plan| !plan.has_drop()) =>
        {
            MirUniformLayout::Native(*id)
        }
        MirErasedType::Reference { inner, .. } | MirErasedType::Pointer(inner)
            if sized(inner, tc) =>
        {
            MirUniformLayout::ThinHandle
        }
        MirErasedType::Tuple(fields) => MirUniformLayout::Tuple(
            fields
                .iter()
                .map(|field| erased_uniform_layout(contract, tc, field))
                .collect::<Result<_, _>>()?,
        ),
        MirErasedType::Array { element, len } => MirUniformLayout::Array {
            element: Box::new(erased_uniform_layout(contract, tc, element)?),
            len: *len,
        },
        _ => return Err(MirErasedError::MissingLayout),
    })
}

pub fn package_plan_is_valid(
    contract: &MirBackendContract,
    tc: &TypeContext,
    plan: &MirPackagePackPlan,
    scopes: &[super::MirErasedScope],
    opened: &[bool],
) -> bool {
    if !package_schema_is_valid(contract, tc, plan.package) {
        return false;
    }
    let schema = &contract.packages[&plan.package];
    let witness = match plan.witness {
        MirPackageWitnessEvidence::Concrete { witness, table } => {
            if !super::object_runtime_type_is_valid(contract, tc, witness)
                || !contract
                    .vtables
                    .get(&table)
                    .is_some_and(|table| table.concrete == witness && table.object == schema.object)
            {
                return false;
            }
            tc.type_for(witness)
        }
        MirPackageWitnessEvidence::Scoped { scope, view } => {
            let Some(evidence) = scopes.get(scope as usize) else {
                return false;
            };
            if opened.get(scope as usize) != Some(&true)
                || view != schema.object
                || !super::object::object_view_is_admitted(tc, evidence.object, view)
            {
                return false;
            }
            Type::Witness(evidence.witness)
        }
    };
    let Ok(payload) = package_payload_type(plan.package, &witness, tc) else {
        return false;
    };
    match plan.payload_descriptor {
        MirPackagePayloadDescriptor::Concrete(id) => {
            matches!(plan.witness, MirPackageWitnessEvidence::Concrete { .. })
                && tc.id_for_type(&payload) == Some(id)
                && contract.erased.descriptors.contains_key(&id)
        }
        MirPackagePayloadDescriptor::UniformDropFree => {
            package_payload_drop_free(contract, tc, plan.package)
        }
        MirPackagePayloadDescriptor::FromPackage(scope) => {
            if !matches!(plan.witness, MirPackageWitnessEvidence::Scoped { scope: witness_scope, .. } if scope == witness_scope)
            {
                return false;
            }
            let Some(evidence) = scopes.get(scope as usize) else {
                return false;
            };
            let super::MirErasedScopeSource::Package { package } = evidence.source else {
                return false;
            };
            package_payload_type(package, &witness, tc).is_ok_and(|original| original == payload)
        }
    }
}
