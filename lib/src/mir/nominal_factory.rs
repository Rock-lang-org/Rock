//! Selected first-order product templates; no runtime implementation search.
use std::collections::{BTreeMap, BTreeSet};

use crate::ids::{DefId, FieldId};
use crate::type_context::TypeContext;
use crate::type_services::kind::Kind;
use crate::types::{NominalTypeKind, Type};

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirNominalDropAuthority { FieldsOnly, UnsupportedUserDrop }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirNominalFactoryRequirement {
    pub descriptor_abi: u64,
    pub result: MirErasedType,
    pub arguments: Vec<MirErasedType>,
    pub fields: Vec<(FieldId, MirErasedType)>,
    pub template: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirBodyFactoryRequirement {
    Tuple(MirTupleFactoryRequirement),
    Nominal(MirNominalFactoryRequirement),
}
impl MirBodyFactoryRequirement {
    pub fn result(&self) -> &MirErasedType {
        match self { Self::Tuple(r) => &r.result, Self::Nominal(r) => &r.result }
    }
    fn dependencies(&self) -> Vec<&MirErasedType> {
        match self {
            Self::Tuple(r) => r.children.iter().collect(),
            Self::Nominal(r) => r.arguments.iter().chain(r.fields.iter().map(|(_, ty)| ty)).collect(),
        }
    }
}

fn frame(out: &mut Vec<u8>, value: &[u8]) {
    out.extend_from_slice(&(value.len() as u64).to_le_bytes()); out.extend_from_slice(value);
}

pub fn nominal_template(contract: &MirBackendContract, tc: &TypeContext, owner: DefId) -> Result<(Vec<u8>, Vec<(FieldId, MirErasedType)>), MirErasedError> {
    let MirNominalLayout::Struct { id, fields, generic_params } = contract.nominal_layouts.get(&owner).ok_or(MirErasedError::MissingLayout)? else {
        return Err(MirErasedError::InvalidType);
    };
    if *id != owner { return Err(MirErasedError::InvalidType); }
    match contract.nominal_drop_authority.get(&owner) {
        Some(MirNominalDropAuthority::FieldsOnly) => {},
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
    Ok(MirNominalFactoryRequirement {
        descriptor_abi: descriptor_identity::DESCRIPTOR_ABI, result: ty.clone(), arguments: args.clone(), template,
        fields: fields.into_iter().map(|(id, ty)| Ok((id, ty.substitute(args)?))).collect::<Result<_, MirErasedError>>()?,
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
        if !scoped.is_empty() { return Ok(()); }
        let requirement = match ty {
            MirErasedType::Tuple(children) => MirBodyFactoryRequirement::Tuple(MirTupleFactoryRequirement { descriptor_abi: descriptor_identity::DESCRIPTOR_ABI, result: ty.clone(), children: children.clone() }),
            MirErasedType::Nominal { .. } => MirBodyFactoryRequirement::Nominal(nominal_requirement(contract, tc, ty)?),
            _ => return Ok(()),
        };
        if active.len() >= 128 || !active.insert(ty.clone()) { return Err(MirErasedError::InvalidType); }
        for child in requirement.dependencies() { visit(child, available, active, out, contract, tc)?; }
        active.remove(ty); available.insert(ty.clone()); out.push(requirement); Ok(())
    }
    let mut available = function.signature.layouts.iter().cloned().collect();
    let mut out = Vec::new(); let mut active = BTreeSet::new();
    if let Some(body) = &function.body {
        for requirement in contract.erased.dictionary_factories.get(&function.key).into_iter().flatten() {
            for ty in &requirement.arguments { visit(ty, &mut available, &mut active, &mut out, contract, tc)?; }
        }
        for ty in &body.locals { visit(ty, &mut available, &mut active, &mut out, contract, tc)?; }
        for statement in body.blocks.iter().flat_map(|block| &block.statements) {
            if let MirErasedRvalue::Call { type_args, .. } | MirErasedRvalue::OpenedErasedCall { type_args, .. } = &statement.value {
                for ty in type_args { visit(ty, &mut available, &mut active, &mut out, contract, tc)?; }
            }
        }
    }
    Ok(out)
}

pub fn validate_body_factories(function: &MirErasedFunction, contract: &MirBackendContract, tc: &TypeContext) -> Result<BTreeSet<MirErasedType>, MirErasedError> {
    let tuples = contract.erased.tuple_factories.get(&function.key).map(Vec::as_slice).unwrap_or(&[]);
    let nominals = contract.erased.nominal_factories.get(&function.key).map(Vec::as_slice).unwrap_or(&[]);
    if tuples.is_empty() && nominals.is_empty() { return Ok(BTreeSet::new()); }
    if function.body.is_none() { return Err(MirErasedError::UnsupportedOperation); }
    // Validate supplied operands before comparing expected requirements, so
    // wrong kind/ordinal/missing dependency retain their strict diagnostics.
    let mut available = function.signature.layouts.iter().cloned().collect::<BTreeSet<_>>();
    let pending = tuples.iter().cloned().map(MirBodyFactoryRequirement::Tuple)
        .chain(nominals.iter().cloned().map(MirBodyFactoryRequirement::Nominal)).collect::<Vec<_>>();
    let mut results = BTreeSet::new();
    for requirement in &pending {
            let abi = match &requirement { MirBodyFactoryRequirement::Tuple(r) => r.descriptor_abi, MirBodyFactoryRequirement::Nominal(r) => r.descriptor_abi };
            if abi != descriptor_identity::DESCRIPTOR_ABI { return Err(MirErasedError::UnsupportedOperation); }
            match &requirement {
                MirBodyFactoryRequirement::Tuple(r) => if !matches!(&r.result, MirErasedType::Tuple(fields) if *fields == r.children) { return Err(MirErasedError::InvalidType); },
                MirBodyFactoryRequirement::Nominal(r) => if *r != nominal_requirement(contract, tc, &r.result)? { return Err(MirErasedError::InvalidType); },
            }
            if available.contains(requirement.result()) || !results.insert(requirement.result().clone()) { return Err(MirErasedError::InvalidType); }
            for child in requirement.dependencies() {
                let mut scoped = BTreeSet::new(); child.scoped_binders(&mut scoped);
                if !scoped.is_empty() { return Err(MirErasedError::UnboundParameter); }
                if child.kind(&function.signature, contract, tc)? != Kind::Type { return Err(MirErasedError::KindMismatch); }
                match child {
                    MirErasedType::Concrete(id) if descriptor_identity::concrete_descriptor_is_sealed(contract, *id) => {},
                    MirErasedType::Parameter(_) | MirErasedType::Tuple(_) | MirErasedType::Nominal { .. } => {},
                    _ => return Err(MirErasedError::MissingLayout),
                }
            }
            if requirement.result().kind(&function.signature, contract, tc)? != Kind::Type { return Err(MirErasedError::KindMismatch); }
    }
    // The two typed lists are ordered subsequences of one body plan. Only
    // their heads may interleave; a reversed list cannot gain dominance by
    // being silently sorted into a valid dependency graph.
    let mut tuple_index = 0;
    let mut nominal_index = tuples.len();
    let mut constructed = BTreeSet::new();
    while tuple_index < tuples.len() || nominal_index < pending.len() {
        let mut progress = false;
        for (index, end) in [(&mut tuple_index, tuples.len()), (&mut nominal_index, pending.len())] {
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
    if tuples != expected_tuples || nominals != expected_nominals { return Err(MirErasedError::InvalidType); }
    Ok(constructed)
}

#[cfg(test)]
#[path = "nominal_factory_tests.rs"]
pub(crate) mod tests;
