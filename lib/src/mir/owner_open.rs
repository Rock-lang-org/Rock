//! Exact selected owner transport in a lexical descriptor environment.
use std::collections::{BTreeMap, HashMap};

use crate::hir::HirImplReceiverPattern;
use crate::type_context::TypeContext;
use crate::types::{NominalTypeKind, Type};

use super::*;

pub(crate) fn validate_rebuild_entry(key: &MirErasedKey, contract: &MirBackendContract, tc: &TypeContext) -> Result<(), MirErasedError> {
    let (implementation, member) = contract.erased.owner_rebuild_entries.get(key).ok_or(MirErasedError::MissingCallable)?;
    let protocol = contract.owner_protocol.as_ref().ok_or(MirErasedError::MissingCallable)?;
    let authority = contract.dictionary_impl_members.get(implementation).ok_or(MirErasedError::MissingCallable)?;
    let entry = contract.erased.functions.get(key).ok_or(MirErasedError::MissingCallable)?;
    if *member != protocol.rebuild || authority.trait_id != protocol.transport
        || authority.members.get(member) != Some(&key.definition)
        || !key.static_args.is_empty() || entry.signature.params.len() != 2
    { return Err(MirErasedError::InvalidType); }
    let MirErasedType::Pointer(payload) = &entry.signature.params[0] else { return Err(MirErasedError::InvalidType); };
    let MirErasedType::Parameter(payload_index) = payload.as_ref() else {
        return Err(MirErasedError::InvalidType);
    };
    let parameter = entry.signature.parameters.get(*payload_index as usize).ok_or(MirErasedError::UnboundParameter)?;
    if parameter.kind != crate::type_services::kind::Kind::Type || !entry.signature.dictionaries.is_empty() {
        return Err(MirErasedError::InvalidType);
    }
    let HirImplReceiverPattern::Constructor(constructor) = contract.owner_constructor_patterns.get(implementation)
        .ok_or(MirErasedError::MissingCallable)? else { return Err(MirErasedError::InvalidType); };
    if crate::type_services::visit::type_any(constructor, |ty| matches!(ty, Type::Generic(id) if *id == parameter.source)) {
        return Err(MirErasedError::InvalidType);
    }
    let applied = tc.normalize_type(&Type::Apply {
        constructor: Box::new(constructor.clone()), args: vec![Type::Generic(parameter.source)],
    }).map_err(|_| MirErasedError::InvalidType)?;
    let variables = entry.signature.parameters.iter().enumerate().map(|(index, parameter)| (parameter.source, index as u32)).collect();
    let expected = MirErasedType::from_type(&applied, &variables, None, &mut tc.clone(), &Default::default())?;
    if !matches!(expected, MirErasedType::Nominal { flavor: NominalTypeKind::Struct, .. })
        || expected != entry.signature.ret
    { return Err(MirErasedError::InvalidType); }
    Ok(())
}

pub(crate) fn operation_types(
    operation: &MirErasedOwnerOperation,
    contract: &MirBackendContract,
) -> Result<(Vec<MirErasedType>, MirErasedType), MirErasedError> {
    let authority = contract.dictionary_impl_members.get(&operation.implementation)
        .ok_or(MirErasedError::MissingCallable)?;
    if authority.trait_id != operation.trait_id
        || authority.members.get(&operation.member) != Some(&operation.definition)
    {
        return Err(MirErasedError::MissingCallable);
    }
    match &operation.target {
        MirErasedOwnerTarget::Native(key) => {
            let callee = contract.callable(key).ok_or(MirErasedError::MissingCallable)?;
            if callee.source_def_id != Some(operation.definition)
                || !matches!(key, MirCallableKey::Instance(_))
                || !matches!(callee.kind, MirCallableKind::LocalBody { .. } | MirCallableKind::ObjectProvided)
                || callee.signature.ret.semantic_ty != callee.signature.ret.abi_ty
            {
                return Err(MirErasedError::InvalidType);
            }
            Ok((callee.signature.params.iter().map(|param| MirErasedType::Concrete(param.semantic_ty)).collect(),
                MirErasedType::Concrete(callee.signature.ret.semantic_ty)))
        }
        MirErasedOwnerTarget::Erased { key, type_args } => {
            let callee = contract.erased.functions.get(key).ok_or(MirErasedError::MissingCallable)?;
            if key.definition != operation.definition || !key.static_args.is_empty()
                || callee.signature.parameters.len() != type_args.len()
                || !callee.signature.dictionaries.is_empty()
            {
                return Err(MirErasedError::InvalidType);
            }
            Ok((callee.signature.params.iter().map(|param| param.substitute(type_args)).collect::<Result<_, _>>()?,
                callee.signature.ret.substitute(type_args)?))
        }
    }
}

pub(crate) fn validate_constructor(
    operation: &MirErasedOwnerOperation,
    contract: &MirBackendContract,
    tc: &TypeContext,
) -> Result<(), MirErasedError> {
    if !tc.type_id_tree_is_valid(operation.constructor) { return Err(MirErasedError::InvalidType); }
    let MirErasedOwnerTarget::Erased { key, type_args } = &operation.target else {
        // A native closed split is paired with the same impl's checked rebuild.
        return Ok(());
    };
    let signature = &contract.erased.functions.get(key).ok_or(MirErasedError::MissingCallable)?.signature;
    let pattern = contract.owner_constructor_patterns.get(&operation.implementation)
        .ok_or(MirErasedError::MissingCallable)?;
    let HirImplReceiverPattern::Constructor(pattern) = pattern else {
        return Err(MirErasedError::UnsupportedOperation);
    };
    let mut bindings = HashMap::new();
    for parameter in signature.parameters.iter().zip(type_args) {
        if crate::type_services::visit::type_any(pattern, |ty| matches!(ty, Type::Generic(id) if *id == parameter.0.source)) {
            bindings.insert(parameter.0.source, parameter.1.instantiate(&[], tc)?);
        }
    }
    let expected = crate::type_services::substitution::instantiate(pattern, &bindings)
        .map_err(|_| MirErasedError::InvalidType)?;
    if tc.normalize_type(&expected).ok().as_ref() != Some(&tc.type_for(operation.constructor)) {
        return Err(MirErasedError::InvalidType);
    }
    Ok(())
}

pub(crate) fn validate_opening(
    opening: &MirErasedOwnerOpening,
    scope: u32,
    evidence: &MirErasedScope,
    contract: &MirBackendContract,
    tc: &TypeContext,
) -> Result<(), MirErasedError> {
    if !tc.type_id_tree_is_valid(opening.closed) || !tc.type_id_tree_is_valid(evidence.object) {
        return Err(MirErasedError::InvalidType);
    }
    let protocol = contract.owner_protocol.as_ref().ok_or(MirErasedError::MissingCallable)?;
    if opening.split.trait_id != protocol.transport || opening.split.member != protocol.split
        || opening.rebuild.trait_id != protocol.transport || opening.rebuild.member != protocol.rebuild
        || opening.split.constructor != opening.rebuild.constructor
        || opening.split.implementation != opening.rebuild.implementation
        || !matches!(opening.split.target, MirErasedOwnerTarget::Native(_))
        || !matches!(opening.rebuild.target, MirErasedOwnerTarget::Erased { .. })
    {
        return Err(MirErasedError::InvalidType);
    }
    validate_constructor(&opening.rebuild, contract, tc)?;
    let constructor = tc.type_for(opening.split.constructor);
    let apply = |payload| match &constructor {
        Type::Constructor { id, flavor: NominalTypeKind::Struct } => Some(Type::Struct { id: *id, args: vec![payload] }),
        Type::Lambda { .. } => crate::type_services::normalize::apply_type_lambda(&constructor, &[payload]),
        _ => None,
    };
    if apply(tc.type_for(evidence.object)).and_then(|ty| tc.normalize_type(&ty).ok()) != Some(tc.type_for(opening.closed)) {
        return Err(MirErasedError::InvalidType);
    }
    let expected = apply(Type::Witness(evidence.witness)).ok_or(MirErasedError::InvalidType)?;
    let expected = MirErasedType::from_type_scoped(&expected, &BTreeMap::new(), None,
        &BTreeMap::from([(evidence.witness, scope)]), &mut tc.clone(), &Default::default())?;
    if expected != opening.opened { return Err(MirErasedError::InvalidType); }
    let (split_params, split_ret) = operation_types(&opening.split, contract)?;
    let (rebuild_params, rebuild_ret) = operation_types(&opening.rebuild, contract)?;
    let pointer = MirErasedType::Pointer(Box::new(MirErasedType::Concrete(evidence.object)));
    let expected_parts = MirErasedType::Tuple(vec![pointer, opening.state.clone()]);
    if split_params.as_slice() != [MirErasedType::Concrete(opening.closed)]
        || !erased_types_agree(&split_ret, &expected_parts, tc)
        || rebuild_params.len() != 2
        || !erased_types_agree(&rebuild_params[0], &MirErasedType::Pointer(Box::new(MirErasedType::Scoped(scope))), tc)
        || !erased_types_agree(&rebuild_params[1], &opening.state, tc)
        || !erased_types_agree(&rebuild_ret, &opening.opened, tc)
    {
        return Err(MirErasedError::InvalidType);
    }
    Ok(())
}

/// These are actual selected operation inputs/results, not arbitrary callee
/// layout claims; the ordinary factory planners revalidate every recipe.
pub(crate) fn transport_types(value: &MirErasedRvalue, contract: &MirBackendContract) -> Result<Vec<MirErasedType>, MirErasedError> {
    let operations = match value {
        MirErasedRvalue::OwnerOpen { plan, .. } => vec![&plan.split, &plan.rebuild],
        MirErasedRvalue::OwnerCall { split, release, .. } => vec![split, release],
        _ => vec![],
    };
    let mut out = Vec::new();
    for operation in operations {
        let (params, ret) = operation_types(operation, contract)?;
        out.extend(params); out.push(ret);
        if let MirErasedOwnerTarget::Erased { type_args, .. } = &operation.target { out.extend(type_args.iter().cloned()); }
    }
    Ok(out)
}

#[cfg(test)]
#[path = "owner_open_tests.rs"]
mod tests;
