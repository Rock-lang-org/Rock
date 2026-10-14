//! Selected first-order product templates; no runtime implementation search.
use std::collections::{BTreeMap, BTreeSet};

use crate::ids::{DefId, FieldId};
use crate::type_context::TypeContext;
use crate::type_services::kind::Kind;
use crate::types::{NominalTypeKind, Type};

use super::*;

#[cfg(test)]
#[path = "thin_handle_factory_tests.rs"]
mod thin_handle_tests;

#[cfg(test)]
#[path = "scoped_factory_tests.rs"]
mod scoped_factory_tests;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirNominalDropAuthority { FieldsOnly, Selected(MirNominalDropSelection), UnsupportedUserDrop }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirNominalDropSelection {
    pub implementation: DefId,
    pub member: DefId,
    pub entry: MirErasedKey,
    /// Entry parameter substitutions in the nominal declaration's binder order.
    pub arguments: Vec<MirErasedType>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirNominalFactoryRequirement {
    pub descriptor_abi: u64,
    pub result: MirErasedType,
    pub arguments: Vec<MirErasedType>,
    pub fields: Vec<(FieldId, MirErasedType)>,
    pub template: Vec<u8>,
    pub user_drop: Option<MirNominalDropSelection>,
}

/// Exact metadata for a thin handle over a descriptor-backed, sized pointee.
/// The pointee supplies identity, not destruction or access permission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirThinHandleFactoryRequirement {
    pub descriptor_abi: u64,
    pub result: MirErasedType,
    pub pointee: MirErasedType,
}

impl MirThinHandleFactoryRequirement {
    pub fn tag(&self) -> Result<u8, MirErasedError> {
        match &self.result {
            MirErasedType::Pointer(inner) if **inner == self.pointee => Ok(b'p'),
            MirErasedType::Reference { inner, mutable } if **inner == self.pointee => {
                Ok(if *mutable { b'R' } else { b'r' })
            }
            _ => Err(MirErasedError::InvalidType),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirBodyFactoryRequirement {
    Tuple(MirTupleFactoryRequirement),
    Nominal(MirNominalFactoryRequirement),
    ThinHandle(MirThinHandleFactoryRequirement),
}
impl MirBodyFactoryRequirement {
    pub fn result(&self) -> &MirErasedType {
        match self { Self::Tuple(r) => &r.result, Self::Nominal(r) => &r.result, Self::ThinHandle(r) => &r.result }
    }
    pub(crate) fn dependencies(&self) -> Vec<&MirErasedType> {
        match self {
            Self::Tuple(r) => r.children.iter().collect(),
            Self::Nominal(r) => r.arguments.iter().chain(r.fields.iter().map(|(_, ty)| ty)).collect(),
            Self::ThinHandle(r) => vec![&r.pointee],
        }
    }
}

fn frame(out: &mut Vec<u8>, value: &[u8]) {
    out.extend_from_slice(&(value.len() as u64).to_le_bytes()); out.extend_from_slice(value);
}

pub(crate) fn nominal_drop_template_key(contract: &MirBackendContract, tc: &TypeContext, selection: &MirNominalDropSelection) -> Result<Vec<u8>, MirErasedError> {
    let definition = descriptor_identity::definition(selection.entry.definition, &contract.descriptor_origins)
        .ok_or(MirErasedError::MissingCallable)?;
    let mut key = b"RU01".to_vec();
    frame(&mut key, &definition);
    key.extend_from_slice(&(selection.arguments.len() as u64).to_le_bytes());
    for argument in &selection.arguments {
        frame(&mut key, &field_expression(contract, tc, argument)?);
    }
    Ok(key)
}

pub fn nominal_template(contract: &MirBackendContract, tc: &TypeContext, owner: DefId) -> Result<(Vec<u8>, Vec<(FieldId, MirErasedType)>), MirErasedError> {
    let MirNominalLayout::Struct { id, fields, generic_params } = contract.nominal_layouts.get(&owner).ok_or(MirErasedError::MissingLayout)? else {
        return Err(MirErasedError::InvalidType);
    };
    if *id != owner { return Err(MirErasedError::InvalidType); }
    match contract.nominal_drop_authority.get(&owner) {
        Some(MirNominalDropAuthority::FieldsOnly | MirNominalDropAuthority::Selected(_)) => {},
        Some(MirNominalDropAuthority::UnsupportedUserDrop) => return Err(MirErasedError::UnsupportedOperation),
        None => return Err(MirErasedError::MissingLayout),
    }
    let ids = contract.nominal_field_ids.get(&owner).ok_or(MirErasedError::MissingLayout)?;
    if ids.len() != fields.len() || ids.iter().collect::<BTreeSet<_>>().len() != ids.len()
        || generic_params.iter().collect::<BTreeSet<_>>().len() != generic_params.len() {
        return Err(MirErasedError::InvalidType);
    }
    let variables = generic_params.iter().enumerate().map(|(i, param)| (*param, i as u32)).collect::<BTreeMap<_, _>>();
    let mut result = b"RN01".to_vec();
    let origin = descriptor_identity::definition(owner, &contract.descriptor_origins).ok_or(MirErasedError::MissingLayout)?;
    frame(&mut result, &origin);
    result.extend_from_slice(&(generic_params.len() as u64).to_le_bytes());
    result.extend_from_slice(&(ids.len() as u64).to_le_bytes());
    let mut templates = Vec::new();
    for (field, (_, ty)) in ids.iter().zip(fields) {
        if !tc.type_id_tree_is_valid(*ty) { return Err(MirErasedError::InvalidType); }
        let erased = MirErasedType::from_type(&tc.type_for(*ty), &variables, None, &mut tc.clone(), &Default::default())?;
        let expression = field_expression(contract, tc, &erased)?;
        result.extend_from_slice(&(field.0 as u64).to_le_bytes()); frame(&mut result, &expression);
        templates.push((*field, erased));
    }
    Ok((result, templates))
}

pub(crate) fn field_expression(contract: &MirBackendContract, tc: &TypeContext, ty: &MirErasedType) -> Result<Vec<u8>, MirErasedError> {
    let (tag, payload, children) = match ty {
        MirErasedType::Parameter(index) => (b'v', (*index as u64).to_le_bytes().to_vec(), vec![]),
        MirErasedType::Concrete(id) => (b'c', descriptor_identity::semantic_identity(contract, tc, *id).ok_or(MirErasedError::MissingLayout)?, vec![]),
        MirErasedType::Tuple(children) => (b't', vec![], children.clone()),
        MirErasedType::Pointer(inner) => (b'p', vec![], vec![*inner.clone()]),
        MirErasedType::Reference { inner, mutable } =>
            (if *mutable { b'R' } else { b'r' }, vec![], vec![*inner.clone()]),
        MirErasedType::Nominal { id, flavor: NominalTypeKind::Struct, args } =>
            (b'S', descriptor_identity::definition(*id, &contract.descriptor_origins).ok_or(MirErasedError::MissingLayout)?, args.clone()),
        _ => return Err(MirErasedError::UnsupportedOperation),
    };
    Ok(descriptor_identity::key(b"RF01", tag, &payload,
        &children.iter().map(|ty| field_expression(contract, tc, ty)).collect::<Result<Vec<_>, _>>()?))
}

pub fn nominal_requirement(contract: &MirBackendContract, tc: &TypeContext, ty: &MirErasedType) -> Result<MirNominalFactoryRequirement, MirErasedError> {
    let MirErasedType::Nominal { id, flavor: NominalTypeKind::Struct, args } = ty else { return Err(MirErasedError::InvalidType); };
    let (template, fields) = nominal_template(contract, tc, *id)?;
    let MirNominalLayout::Struct { generic_params, .. } = &contract.nominal_layouts[id] else { return Err(MirErasedError::InvalidType); };
    if args.len() != generic_params.len() { return Err(MirErasedError::InvalidType); }
    let user_drop = match contract.nominal_drop_authority.get(id) {
        Some(MirNominalDropAuthority::Selected(selection)) => {
            if contract.erased.nominal_drop_entries.get(id) != Some(selection)
                || !contract.dictionary_impl_members.get(&selection.implementation).is_some_and(|authority| {
                    authority.members.get(&selection.member) == Some(&selection.entry.definition)
                })
            {
                return Err(MirErasedError::MissingCallable);
            }
            let mut selected = selection.clone();
            selected.arguments = selected.arguments.iter().map(|ty| ty.substitute(args)).collect::<Result<_, _>>()?;
            let function = contract.erased.functions.get(&selected.entry).ok_or(MirErasedError::MissingCallable)?;
            if !function.signature.dictionaries.is_empty()
                || function.signature.parameters.len() != selected.arguments.len()
                || function.signature.params.len() != 1
                || function.signature.params[0].substitute(&selected.arguments)? != *ty
                || !matches!(function.signature.ret, MirErasedType::Concrete(ret) if tc.type_for(ret) == Type::Unit)
            {
                return Err(MirErasedError::InvalidType);
            }
            Some(selected)
        }
        _ => None,
    };
    Ok(MirNominalFactoryRequirement {
        descriptor_abi: descriptor_identity::DESCRIPTOR_ABI, result: ty.clone(), arguments: args.clone(), template,
        fields: fields.into_iter().map(|(id, ty)| Ok((id, ty.substitute(args)?))).collect::<Result<_, MirErasedError>>()?,
        user_drop,
    })
}

pub fn nominal_field(contract: &MirBackendContract, tc: &TypeContext, ty: &MirErasedType, owner: DefId, field: FieldId) -> Result<(u32, MirErasedType), MirErasedError> {
    let actual = match ty {
        MirErasedType::Nominal { id, .. } => *id,
        MirErasedType::Concrete(id) => match tc.type_for(*id) { Type::Struct { id, .. } => id, _ => return Err(MirErasedError::InvalidType) },
        _ => return Err(MirErasedError::InvalidType),
    };
    if actual != owner { return Err(MirErasedError::InvalidType); }
    let ids = contract.nominal_field_ids.get(&owner).ok_or(MirErasedError::MissingLayout)?;
    let MirNominalLayout::Struct { id, fields, .. } = contract.nominal_layouts.get(&owner).ok_or(MirErasedError::MissingLayout)? else {
        return Err(MirErasedError::InvalidType);
    };
    if *id != owner || ids.len() != fields.len() || ids.iter().collect::<BTreeSet<_>>().len() != ids.len() {
        return Err(MirErasedError::InvalidType);
    }
    let index = ids.iter().position(|id| *id == field).ok_or(MirErasedError::InvalidType)?;
    let field_ty = match ty {
        MirErasedType::Concrete(id) => MirErasedType::Concrete(*contract.erased.descriptors.get(id).ok_or(MirErasedError::MissingLayout)?.fields.get(index).ok_or(MirErasedError::InvalidType)?),
        _ => nominal_requirement(contract, tc, ty)?.fields.get(index).ok_or(MirErasedError::InvalidType)?.1.clone(),
    };
    Ok((u32::try_from(index).map_err(|_| MirErasedError::InvalidType)?, field_ty))
}

pub fn collect_body_factories(function: &MirErasedFunction, contract: &MirBackendContract, tc: &TypeContext) -> Result<Vec<MirBodyFactoryRequirement>, MirErasedError> {
    fn visit(ty: &MirErasedType, available: &mut BTreeSet<MirErasedType>, active: &mut BTreeSet<MirErasedType>, out: &mut Vec<MirBodyFactoryRequirement>, contract: &MirBackendContract, tc: &TypeContext) -> Result<(), MirErasedError> {
        if available.contains(ty) { return Ok(()); }
        let mut scoped = BTreeSet::new(); ty.scoped_binders(&mut scoped);
        let requirement = match ty {
            MirErasedType::Tuple(children) => MirBodyFactoryRequirement::Tuple(MirTupleFactoryRequirement { descriptor_abi: descriptor_identity::DESCRIPTOR_ABI, result: ty.clone(), children: children.clone() }),
            MirErasedType::Nominal { .. } => MirBodyFactoryRequirement::Nominal(nominal_requirement(contract, tc, ty)?),
            MirErasedType::Pointer(inner) | MirErasedType::Reference { inner, .. } =>
                MirBodyFactoryRequirement::ThinHandle(MirThinHandleFactoryRequirement {
                    descriptor_abi: descriptor_identity::DESCRIPTOR_ABI,
                    result: ty.clone(),
                    pointee: *inner.clone(),
                }),
            _ => return Ok(()),
        };
        if active.len() >= 128 || !active.insert(ty.clone()) { return Err(MirErasedError::InvalidType); }
        for child in requirement.dependencies() { visit(child, available, active, out, contract, tc)?; }
        active.remove(ty);
        if scoped.is_empty() { available.insert(ty.clone()); out.push(requirement); }
        Ok(())
    }
    let mut available = function.signature.layouts.iter().cloned().collect::<BTreeSet<_>>();
    let mut out = Vec::new(); let mut active = BTreeSet::new();
    if let Some(body) = &function.body {
        for (index, scope) in body.scopes.iter().enumerate() {
            if let MirErasedScopeSource::Package { package } = scope.source {
                available.insert(scoped_package_payload(package, index as u32, scope.witness, &mut tc.clone())?);
            }
        }
        for requirement in contract.erased.dictionary_factories.get(&function.key).into_iter().flatten() {
            for ty in &requirement.arguments { visit(ty, &mut available, &mut active, &mut out, contract, tc)?; }
        }
        for ty in &body.locals { visit(ty, &mut available, &mut active, &mut out, contract, tc)?; }
        for statement in body.blocks.iter().flat_map(|block| &block.statements) {
            for ty in super::owner_open::transport_types(&statement.value, contract)? {
                visit(&ty, &mut available, &mut active, &mut out, contract, tc)?;
            }
            if let MirErasedRvalue::Call { type_args, .. } | MirErasedRvalue::OpenedErasedCall { type_args, .. } = &statement.value {
                for ty in type_args { visit(ty, &mut available, &mut active, &mut out, contract, tc)?; }
            }
            if let MirErasedRvalue::Call { args, .. } = &statement.value {
                // A projected argument may need exact metadata without having
                // its own local. Its checked place, not a callee's extra layout
                // claims, supplies that construction authority.
                for operand in args {
                    let (MirErasedOperand::Move(place) | MirErasedOperand::Copy(place)) = operand;
                    if !place.projection.is_empty()
                        && !matches!(body.locals.get(place.local.0 as usize), Some(MirErasedType::Concrete(_)))
                    {
                        visit(&erased_place_type(function, place, contract, tc)?, &mut available, &mut active, &mut out, contract, tc)?;
                    }
                }
            }
        }
    }
    Ok(out)
}

pub fn validate_body_factories(function: &MirErasedFunction, contract: &MirBackendContract, tc: &TypeContext) -> Result<BTreeSet<MirErasedType>, MirErasedError> {
    let tuples = contract.erased.tuple_factories.get(&function.key).map(Vec::as_slice).unwrap_or(&[]);
    let nominals = contract.erased.nominal_factories.get(&function.key).map(Vec::as_slice).unwrap_or(&[]);
    let handles = contract.erased.thin_handle_factories.get(&function.key).map(Vec::as_slice).unwrap_or(&[]);
    if tuples.is_empty() && nominals.is_empty() && handles.is_empty() { return Ok(BTreeSet::new()); }
    if function.body.is_none() { return Err(MirErasedError::UnsupportedOperation); }
    // Validate supplied operands before comparing expected requirements, so
    // wrong kind/ordinal/missing dependency retain their strict diagnostics.
    let mut available = function.signature.layouts.iter().cloned().collect::<BTreeSet<_>>();
    let pending = tuples.iter().cloned().map(MirBodyFactoryRequirement::Tuple)
        .chain(nominals.iter().cloned().map(MirBodyFactoryRequirement::Nominal))
        .chain(handles.iter().cloned().map(MirBodyFactoryRequirement::ThinHandle)).collect::<Vec<_>>();
    let mut results = BTreeSet::new();
    for requirement in &pending {
            let abi = match &requirement { MirBodyFactoryRequirement::Tuple(r) => r.descriptor_abi, MirBodyFactoryRequirement::Nominal(r) => r.descriptor_abi, MirBodyFactoryRequirement::ThinHandle(r) => r.descriptor_abi };
            if abi != descriptor_identity::DESCRIPTOR_ABI { return Err(MirErasedError::UnsupportedOperation); }
            match &requirement {
                MirBodyFactoryRequirement::Tuple(r) => if !matches!(&r.result, MirErasedType::Tuple(fields) if *fields == r.children) { return Err(MirErasedError::InvalidType); },
                MirBodyFactoryRequirement::Nominal(r) => if *r != nominal_requirement(contract, tc, &r.result)? { return Err(MirErasedError::InvalidType); },
                MirBodyFactoryRequirement::ThinHandle(r) => { r.tag()?; },
            }
            if available.contains(requirement.result()) || !results.insert(requirement.result().clone()) { return Err(MirErasedError::InvalidType); }
            for child in requirement.dependencies() {
                let mut scoped = BTreeSet::new(); child.scoped_binders(&mut scoped);
                if !scoped.is_empty() { return Err(MirErasedError::UnboundParameter); }
                if child.kind(&function.signature, contract, tc)? != Kind::Type { return Err(MirErasedError::KindMismatch); }
                match child {
                    MirErasedType::Concrete(id) if descriptor_identity::concrete_descriptor_is_sealed(contract, *id) => {},
                    MirErasedType::Parameter(_) | MirErasedType::Tuple(_) | MirErasedType::Nominal { .. }
                    | MirErasedType::Pointer(_) | MirErasedType::Reference { .. } => {},
                    _ => return Err(MirErasedError::MissingLayout),
                }
            }
            if requirement.result().kind(&function.signature, contract, tc)? != Kind::Type { return Err(MirErasedError::KindMismatch); }
    }
    // The typed lists are ordered subsequences of one body plan. Only
    // their heads may interleave; a reversed list cannot gain dominance by
    // being silently sorted into a valid dependency graph.
    let mut tuple_index = 0;
    let mut nominal_index = tuples.len();
    let mut handle_index = tuples.len() + nominals.len();
    let mut constructed = BTreeSet::new();
    while tuple_index < tuples.len() || nominal_index < tuples.len() + nominals.len() || handle_index < pending.len() {
        let mut progress = false;
        for (index, end) in [(&mut tuple_index, tuples.len()), (&mut nominal_index, tuples.len() + nominals.len()), (&mut handle_index, pending.len())] {
            if *index >= end { continue; }
            let requirement = &pending[*index];
            if requirement.dependencies().iter().all(|child| matches!(child, MirErasedType::Concrete(_)) || available.contains(*child)) {
                available.insert(requirement.result().clone());
                constructed.insert(requirement.result().clone());
                *index += 1;
                progress = true;
            }
        }
        if !progress { return Err(MirErasedError::MissingLayout); }
    }
    let expected = collect_body_factories(function, contract, tc)?;
    let expected_tuples = expected.iter().filter_map(|r| if let MirBodyFactoryRequirement::Tuple(r) = r { Some(r.clone()) } else { None }).collect::<Vec<_>>();
    let expected_nominals = expected.iter().filter_map(|r| if let MirBodyFactoryRequirement::Nominal(r) = r { Some(r.clone()) } else { None }).collect::<Vec<_>>();
    let expected_handles = expected.iter().filter_map(|r| if let MirBodyFactoryRequirement::ThinHandle(r) = r { Some(r.clone()) } else { None }).collect::<Vec<_>>();
    if tuples != expected_tuples || nominals != expected_nominals || handles != expected_handles { return Err(MirErasedError::InvalidType); }
    Ok(constructed)
}

/// Reify exact products after their dominating Open; forwarding alone does not
/// construct metadata or replace the payload descriptor carried by a package.
pub fn collect_scoped_factories(
    function: &MirErasedFunction,
    contract: &MirBackendContract,
    tc: &TypeContext,
) -> Result<BTreeMap<u32, Vec<MirBodyFactoryRequirement>>, MirErasedError> {
    let mut exact = contract.erased.functions.keys().filter(|key| {
        contract.erased.tuple_factories.get(*key).is_some_and(|r| !r.is_empty())
            || contract.erased.nominal_factories.get(*key).is_some_and(|r| !r.is_empty())
            || contract.erased.thin_handle_factories.get(*key).is_some_and(|r| !r.is_empty())
    }).cloned().collect::<BTreeSet<_>>();
    loop {
        let before = exact.len();
        for (key, entry) in &contract.erased.functions {
            if entry.body.as_ref().is_some_and(|body| {
                body.blocks.iter().flat_map(|block| &block.statements).any(|statement| {
                    matches!(&statement.value, MirErasedRvalue::Call { target, .. } if exact.contains(target))
                })
            }) {
                exact.insert(key.clone());
            }
        }
        if exact.len() == before { break; }
    }
    fn visit(
        ty: &MirErasedType,
        scopes: &[MirErasedScope],
        seen: &mut BTreeSet<MirErasedType>,
        out: &mut BTreeMap<u32, Vec<MirBodyFactoryRequirement>>,
        contract: &MirBackendContract,
        tc: &TypeContext,
        depth: usize,
    ) -> Result<(), MirErasedError> {
        let mut binders = BTreeSet::new();
        ty.scoped_binders(&mut binders);
        if binders.is_empty() || seen.contains(ty) { return Ok(()); }
        if depth >= 128 { return Err(MirErasedError::UnsupportedOperation); }
        if binders.len() != 1 {
            return Err(MirErasedError::UnsupportedOperation);
        }
        let scope = *binders.first().ok_or(MirErasedError::UnboundParameter)?;
        if scope as usize >= scopes.len() { return Err(MirErasedError::UnboundParameter); }
        let requirement = match ty {
            MirErasedType::Scoped(_) => return Ok(()),
            MirErasedType::Pointer(inner) | MirErasedType::Reference { inner, .. } =>
                MirBodyFactoryRequirement::ThinHandle(MirThinHandleFactoryRequirement {
                    descriptor_abi: descriptor_identity::DESCRIPTOR_ABI,
                    result: ty.clone(), pointee: *inner.clone(),
                }),
            MirErasedType::Tuple(children) => MirBodyFactoryRequirement::Tuple(MirTupleFactoryRequirement {
                descriptor_abi: descriptor_identity::DESCRIPTOR_ABI,
                result: ty.clone(), children: children.clone(),
            }),
            MirErasedType::Nominal { .. } => MirBodyFactoryRequirement::Nominal(nominal_requirement(contract, tc, ty)?),
            _ => return Err(MirErasedError::MissingLayout),
        };
        for child in requirement.dependencies() { visit(child, scopes, seen, out, contract, tc, depth + 1)?; }
        seen.insert(ty.clone());
        out.entry(scope).or_default().push(requirement);
        Ok(())
    }
    let mut out = BTreeMap::new();
    let mut seen = BTreeSet::new();
    if let Some(body) = &function.body {
        for (index, scope) in body.scopes.iter().enumerate() {
            if let MirErasedScopeSource::Package { package } = scope.source {
                seen.insert(scoped_package_payload(package, index as u32, scope.witness, &mut tc.clone())?);
            }
        }
        for ty in &body.locals {
            // Proven uniform drop-free products need no synthesized identity;
            // exact callee demands below still select their metadata explicitly.
            if matches!(ty, MirErasedType::Nominal { .. } | MirErasedType::Tuple(_))
                && erased_uniform_layout(contract, &mut tc.clone(), ty).is_err()
            {
                visit(ty, &body.scopes, &mut seen, &mut out, contract, tc, 0)?;
            }
        }
        for statement in body.blocks.iter().flat_map(|block| &block.statements) {
            for ty in super::owner_open::transport_types(&statement.value, contract)? {
                visit(&ty, &body.scopes, &mut seen, &mut out, contract, tc, 0)?;
            }
            if let MirErasedRvalue::Call { target, type_args, .. } = &statement.value {
                if exact.contains(target) {
                    for ty in type_args { visit(ty, &body.scopes, &mut seen, &mut out, contract, tc, 0)?; }
                }
            }
        }
    }
    Ok(out)
}

pub fn validate_scoped_factories(
    function: &MirErasedFunction,
    contract: &MirBackendContract,
    tc: &TypeContext,
) -> Result<(), MirErasedError> {
    let supplied = contract.erased.scoped_factories.get(&function.key).cloned().unwrap_or_default();
    if !supplied.is_empty() && function.body.is_none() { return Err(MirErasedError::UnsupportedOperation); }
    for handles in supplied.values() {
        if handles.is_empty() { return Err(MirErasedError::InvalidType); }
        for handle in handles {
            let abi = match handle {
                MirBodyFactoryRequirement::ThinHandle(handle) => { handle.tag()?; handle.descriptor_abi },
                MirBodyFactoryRequirement::Tuple(tuple) => tuple.descriptor_abi,
                MirBodyFactoryRequirement::Nominal(nominal) => nominal.descriptor_abi,
            };
            if abi != descriptor_identity::DESCRIPTOR_ABI { return Err(MirErasedError::UnsupportedOperation); }
            if handle.result().kind(&function.signature, contract, tc)? != Kind::Type { return Err(MirErasedError::KindMismatch); }
        }
    }
    if supplied != collect_scoped_factories(function, contract, tc)? { return Err(MirErasedError::InvalidType); }
    Ok(())
}

#[cfg(test)]
#[path = "nominal_factory_tests.rs"]
pub(crate) mod tests;
