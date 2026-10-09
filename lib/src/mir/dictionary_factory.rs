//! Compile-time selected evidence and exact, payload-free dictionary identities.
use std::collections::BTreeSet;

use crate::ids::DefId;
use crate::type_context::TypeContext;
use crate::type_services::kind::Kind;
use crate::types::ReceiverMode;

use super::*;

pub const DICTIONARY_ABI: u64 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirDictionaryImplAuthority {
    pub trait_id: DefId,
    pub members: std::collections::BTreeMap<DefId, DefId>,
    pub factory_compatible: bool,
    pub template: Option<MirDictionaryFactoryTemplate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirDictionaryFactoryMember {
    pub member_id: DefId,
    pub method_id: DefId,
    pub entry: MirErasedKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirDictionaryFactoryTemplate {
    pub implementation: DefId,
    pub parameters: Vec<MirErasedParameter>,
    pub result: MirErasedDictionarySchema,
    pub bounds: Vec<MirErasedDictionarySchema>,
    pub members: Vec<MirDictionaryFactoryMember>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirDictionaryFactoryRequirement {
    pub implementation: DefId,
    pub arguments: Vec<MirErasedType>,
    pub children: Vec<u32>,
    pub result: MirErasedDictionarySchema,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirDictionaryWitness {
    pub implementation: DefId,
    pub schema: MirErasedDictionarySchema,
    pub methods: Vec<DefId>,
}

pub(crate) fn dictionary_frame(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    out.extend_from_slice(bytes);
}

fn dictionary_borrow_key(borrowing: &crate::hir::borrowing::BorrowContract) -> Vec<u8> {
    use crate::hir::borrowing::{BorrowPath, BorrowProjection, BorrowSource, BorrowSourceKind};
    fn path(out: &mut Vec<u8>, path: &BorrowPath) {
        let mut bytes = (path.len() as u64).to_le_bytes().to_vec();
        for projection in path {
            let tag: u64 = match projection {
                BorrowProjection::Field(_) => 0,
                BorrowProjection::Element => 1,
                BorrowProjection::Deref => 2,
            };
            bytes.extend_from_slice(&tag.to_le_bytes());
            if let BorrowProjection::Field(index) = projection {
                bytes.extend_from_slice(&(*index as u64).to_le_bytes());
            }
        }
        dictionary_frame(out, &bytes);
    }
    fn source(out: &mut Vec<u8>, source: &BorrowSource) {
        out.extend_from_slice(&(source.argument as u64).to_le_bytes());
        out.extend_from_slice(
            &(match source.kind {
                BorrowSourceKind::Value => 0u64,
                BorrowSourceKind::Contents => 1,
            })
            .to_le_bytes(),
        );
        path(out, &source.path);
    }
    let mut out = b"BQ01".to_vec();
    out.extend_from_slice(&(borrowing.returns.len() as u64).to_le_bytes());
    for effect in &borrowing.returns {
        path(&mut out, &effect.path);
        out.extend_from_slice(&(effect.sources.len() as u64).to_le_bytes());
        for origin in &effect.sources {
            source(&mut out, origin);
        }
    }
    out.extend_from_slice(&(borrowing.retentions.len() as u64).to_le_bytes());
    for effect in &borrowing.retentions {
        source(&mut out, &effect.destination);
        out.extend_from_slice(&(effect.sources.len() as u64).to_le_bytes());
        for origin in &effect.sources {
            source(&mut out, origin);
        }
    }
    out
}

/// Conservative return origins exist even for scalar/unit results; only the
/// result's actual semantic shape proves this bounded factory cannot lend.
fn dictionary_result_supported(ty: &MirErasedType, tc: &TypeContext) -> bool {
    fn supported(ty: &crate::types::Type) -> bool {
        use crate::types::Type;
        match ty {
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
            | Type::Str
            | Type::Unit
            | Type::Never
            | Type::Pointer(_) => true,
            Type::Tuple(fields) => fields.iter().all(supported),
            Type::Array(element, _) => supported(element),
            _ => false,
        }
    }
    ty.instantiate(&[], tc).is_ok_and(|ty| supported(&ty))
}

/// Closed members use scalar/non-borrowing results; nested Type binders preserve
/// the complete parametric input-origin envelope, including actual loan values.
pub fn dictionary_schema_key(
    contract: &MirBackendContract,
    tc: &TypeContext,
    schema: &MirErasedDictionarySchema,
    instantiated: bool,
) -> Result<Vec<u8>, MirErasedError> {
    let mut out = if instantiated {
        b"DK01".to_vec()
    } else {
        b"DS01".to_vec()
    };
    out.extend_from_slice(&DICTIONARY_ABI.to_le_bytes());
    let definition = |id| {
        descriptor_identity::definition(id, &contract.descriptor_origins)
            .ok_or(MirErasedError::InvalidDictionary)
    };
    dictionary_frame(&mut out, &definition(schema.trait_id)?);
    let encode = |ty: &MirErasedType| {
        let _ = ty.map_types(&mut |id| {
            if tc.type_id_tree_is_valid(*id) {
                Ok(*id)
            } else {
                Err(MirErasedError::InvalidType)
            }
        })?;
        if instantiated {
            let ty = ty.instantiate(&[], tc)?;
            let id = tc.id_for_type(&ty).ok_or(MirErasedError::MissingLayout)?;
            descriptor_identity::semantic_identity(contract, tc, id)
                .ok_or(MirErasedError::MissingLayout)
        } else {
            super::nominal_factory::field_expression(contract, tc, ty)
        }
    };
    dictionary_frame(&mut out, &encode(&schema.subject)?);
    out.extend_from_slice(&(schema.trait_args.len() as u64).to_le_bytes());
    for arg in &schema.trait_args {
        dictionary_frame(&mut out, &encode(arg)?);
    }
    out.extend_from_slice(&(schema.members.len() as u64).to_le_bytes());
    let mut ids = BTreeSet::new();
    for member in &schema.members {
        if !ids.insert(member.member_id)
            || (member.receiver.is_some() && member.params.first() != Some(&schema.subject))
            || member.borrowing
                != super::dictionary_member_borrow_contract(
                    &member.binder,
                    &member.params,
                    &member.ret,
                    member.receiver,
                    tc,
                )?
        {
            return Err(MirErasedError::InvalidDictionary);
        }
        let static_shape = member.receiver.is_none()
            && member.binder.parameters.is_empty()
            && static_dictionary_shape_supported(&member.params, &member.ret, &schema.subject, tc);
        if (member.receiver.is_none() && !static_shape)
            || (member.binder.parameters.is_empty()
                && !static_shape
                && !dictionary_result_supported(&member.ret, tc))
            || !member.borrowing.retentions.is_empty()
        {
            return Err(MirErasedError::UnsupportedOperation);
        }
        dictionary_frame(&mut out, &definition(member.member_id)?);
        out.extend_from_slice(
            &(match member.receiver {
                Some(ReceiverMode::Shared) => 0u64,
                Some(ReceiverMode::Mut) => 1,
                Some(ReceiverMode::Move) => 2,
                None => 3,
            })
            .to_le_bytes(),
        );
        out.extend_from_slice(&(member.binder.parameters.len() as u64).to_le_bytes());
        for parameter in &member.binder.parameters {
            if parameter.kind != Kind::Type {
                return Err(MirErasedError::KindMismatch);
            }
            out.extend_from_slice(&0u64.to_le_bytes());
        }
        if !member.binder.dictionaries.is_empty() {
            return Err(MirErasedError::UnsupportedOperation);
        }
        out.extend_from_slice(&0u64.to_le_bytes());
        out.extend_from_slice(&(member.params.len() as u64).to_le_bytes());
        let member_encode = |ty: &MirErasedType| {
            if member.binder.parameters.is_empty() {
                encode(ty)
            } else {
                super::polymorphic_dictionary::member_type_key(
                    ty,
                    member.binder.parameters.len(),
                    &encode,
                )
            }
        };
        for param in &member.params {
            dictionary_frame(&mut out, &member_encode(param)?);
        }
        dictionary_frame(&mut out, &member_encode(&member.ret)?);
        dictionary_frame(&mut out, &dictionary_borrow_key(&member.borrowing));
    }
    Ok(out)
}

pub fn dictionary_template_key(
    contract: &MirBackendContract,
    tc: &TypeContext,
    template: &MirDictionaryFactoryTemplate,
) -> Result<Vec<u8>, MirErasedError> {
    if template
        .parameters
        .iter()
        .any(|parameter| parameter.kind != Kind::Type)
        || template
            .parameters
            .iter()
            .map(|parameter| parameter.source)
            .collect::<BTreeSet<_>>()
            .len()
            != template.parameters.len()
    {
        return Err(MirErasedError::KindMismatch);
    }
    for schema in std::iter::once(&template.result).chain(&template.bounds) {
        for ty in std::iter::once(&schema.subject).chain(&schema.trait_args) {
            super::validate_dictionary_binder_scope(ty, template.parameters.len(), 0)?;
        }
        for member in &schema.members {
            member.entry_signature(&template.parameters)?;
        }
    }
    let authority = contract
        .dictionary_impl_members
        .get(&template.implementation)
        .ok_or(MirErasedError::InvalidDictionary)?;
    if !authority.factory_compatible || authority.trait_id != template.result.trait_id {
        return Err(MirErasedError::InvalidDictionary);
    }
    if !template.parameters.is_empty() && authority.template.as_ref() != Some(template) {
        return Err(MirErasedError::InvalidDictionary);
    }
    let mut out = b"DQ01".to_vec();
    dictionary_frame(
        &mut out,
        &descriptor_identity::definition(template.implementation, &contract.descriptor_origins)
            .ok_or(MirErasedError::InvalidDictionary)?,
    );
    out.extend_from_slice(&(template.parameters.len() as u64).to_le_bytes());
    dictionary_frame(
        &mut out,
        &dictionary_schema_key(contract, tc, &template.result, false)?,
    );
    out.extend_from_slice(&(template.bounds.len() as u64).to_le_bytes());
    for bound in &template.bounds {
        dictionary_frame(
            &mut out,
            &dictionary_schema_key(contract, tc, bound, false)?,
        );
    }
    if template.members.len() != template.result.members.len() {
        return Err(MirErasedError::InvalidDictionary);
    }
    out.extend_from_slice(&(template.members.len() as u64).to_le_bytes());
    for (binding, member) in template.members.iter().zip(&template.result.members) {
        if binding.member_id != member.member_id
            || authority.members.get(&binding.member_id) != Some(&binding.method_id)
        {
            return Err(MirErasedError::InvalidDictionary);
        }
        dictionary_frame(
            &mut out,
            &descriptor_identity::definition(binding.method_id, &contract.descriptor_origins)
                .ok_or(MirErasedError::InvalidDictionary)?,
        );
    }
    Ok(out)
}

pub fn native_dictionary_keys(
    contract: &MirBackendContract,
    tc: &TypeContext,
    id: MirDictionaryId,
) -> Result<(Vec<u8>, Vec<u8>), MirErasedError> {
    let dictionary = contract
        .erased
        .dictionaries
        .get(&id)
        .ok_or(MirErasedError::InvalidDictionary)?;
    let witness = contract
        .erased
        .dictionary_witnesses
        .get(&id)
        .ok_or(MirErasedError::InvalidDictionary)?;
    if !tc.type_id_tree_is_valid(dictionary.subject)
        || dictionary
            .trait_args
            .iter()
            .any(|id| !tc.type_id_tree_is_valid(*id))
    {
        return Err(MirErasedError::InvalidDictionary);
    }
    if witness.schema.subject != MirErasedType::Concrete(dictionary.subject)
        || witness.schema.trait_id != dictionary.trait_id
        || witness.schema.trait_args
            != dictionary
                .trait_args
                .iter()
                .copied()
                .map(MirErasedType::Concrete)
                .collect::<Vec<_>>()
        || witness.schema.members.len() != dictionary.members.len()
        || witness.methods.len() != dictionary.members.len()
    {
        return Err(MirErasedError::InvalidDictionary);
    }
    let mut members = Vec::new();
    for ((actual, schema), method) in dictionary
        .members
        .iter()
        .zip(&witness.schema.members)
        .zip(&witness.methods)
    {
        if let MirDictionaryMemberTarget::Polymorphic(key) = &actual.target {
            super::validate_member_entry(key, contract, tc)?;
            let entry = &contract.erased.member_entries[key];
            if !entry.captures.is_empty()
                || entry.implementation != witness.implementation
                || entry.trait_id != dictionary.trait_id
                || entry.member != *schema
                || entry.method_id != *method
                || actual.member_id != schema.member_id
                || actual.receiver != schema.receiver
            {
                return Err(MirErasedError::InvalidDictionary);
            }
            members.push(MirDictionaryFactoryMember {
                member_id: actual.member_id,
                method_id: *method,
                entry: key.clone(),
            });
            continue;
        }
        let callable = contract
            .callable(
                actual
                    .target
                    .native()
                    .ok_or(MirErasedError::InvalidDictionary)?,
            )
            .ok_or(MirErasedError::MissingCallable)?;
        let result_storage = contract.callable_result_storage(callable);
        let types = callable
            .signature
            .params
            .iter()
            .map(|param| param.semantic_ty)
            .collect::<Vec<_>>();
        if types
            .iter()
            .chain([
                &callable.signature.ret.semantic_ty,
                &callable.signature.ret.abi_ty,
                &result_storage,
            ])
            .any(|id| !tc.type_id_tree_is_valid(*id))
        {
            return Err(MirErasedError::InvalidDictionary);
        }
        if callable.signature
            != MirCallableSignature::from_instance_type_ids(
                &types,
                callable.signature.ret.semantic_ty,
                schema.receiver.is_some(),
                tc,
            )
        {
            return Err(MirErasedError::InvalidDictionary);
        }
        if callable.source_def_id != Some(*method)
            || actual.member_id != schema.member_id
            || actual.receiver != schema.receiver
            || (schema.receiver.is_some() && schema.params.is_empty())
            || callable.signature.params.len() != schema.params.len()
        {
            return Err(MirErasedError::InvalidDictionary);
        }
        let mut params = callable
            .signature
            .params
            .iter()
            .map(|p| MirErasedType::Concrete(p.semantic_ty))
            .collect::<Vec<_>>();
        if let Some(mode) = schema.receiver {
            let receiver = tc.type_for(callable.signature.params[0].semantic_ty);
            let subject = tc.type_for(dictionary.subject);
            let expected_receiver = match mode {
                ReceiverMode::Move => subject,
                mode => crate::types::Type::Reference {
                    inner: Box::new(subject),
                    mutable: mode == ReceiverMode::Mut,
                },
            };
            if receiver != expected_receiver {
                return Err(MirErasedError::InvalidDictionary);
            }
            params[0] = MirErasedType::Concrete(dictionary.subject);
        }
        let declared_result = schema.ret.instantiate(&[], tc)?;
        let storage = tc.type_for(result_storage);
        // A nominal Self witness is not a storage assertion. Invocations still
        // check the producer's exact storage; all nonnominal results stay exact.
        let canonical_nominal_result = schema.receiver.is_none()
            && schema.ret == witness.schema.subject
            && matches!(
                &declared_result,
                crate::types::Type::Struct { .. } | crate::types::Type::Enum { .. }
            )
            && declared_result == contract.normalize_type(tc, &storage);
        if params != schema.params || (declared_result != storage && !canonical_nominal_result) {
            return Err(MirErasedError::InvalidDictionary);
        }
        members.push(MirDictionaryFactoryMember {
            member_id: actual.member_id,
            method_id: *method,
            entry: MirErasedKey {
                definition: *method,
                static_args: vec![],
            },
        });
    }
    let template = MirDictionaryFactoryTemplate {
        implementation: witness.implementation,
        parameters: vec![],
        result: witness.schema.clone(),
        bounds: vec![],
        members,
    };
    let schema = dictionary_schema_key(contract, tc, &witness.schema, true)?;
    let mut binding = b"DI01".to_vec();
    dictionary_frame(&mut binding, &schema);
    dictionary_frame(
        &mut binding,
        &dictionary_template_key(contract, tc, &template)?,
    );
    binding.extend_from_slice(&0u64.to_le_bytes());
    binding.extend_from_slice(&0u64.to_le_bytes());
    Ok((schema, binding))
}

pub fn validate_dictionary_factories(
    function: &MirErasedFunction,
    contract: &MirBackendContract,
    tc: &TypeContext,
    layouts: &BTreeSet<MirErasedType>,
) -> Result<Vec<MirErasedDictionarySchema>, MirErasedError> {
    let mut available = function.signature.dictionaries.clone();
    let requirements = contract
        .erased
        .dictionary_factories
        .get(&function.key)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if !requirements.is_empty() && function.body.is_none() {
        return Err(MirErasedError::UnsupportedOperation);
    }
    let mut used = BTreeSet::new();
    if let Some(body) = &function.body {
        for statement in body.blocks.iter().flat_map(|block| &block.statements) {
            match &statement.value {
                MirErasedRvalue::Call { dictionaries, .. }
                | MirErasedRvalue::OpenedErasedCall { dictionaries, .. } => {
                    used.extend(dictionaries.iter().copied())
                }
                MirErasedRvalue::DictionaryCall { dictionary, .. } => {
                    used.insert(*dictionary);
                }
                _ => {}
            }
        }
    }
    for (index, requirement) in requirements.iter().enumerate().rev() {
        // dependencies are also observed roots
        if used.contains(&((available.len() + index) as u32)) {
            used.extend(requirement.children.iter().copied());
        }
    }
    for requirement in requirements {
        let template = contract
            .erased
            .dictionary_templates
            .get(&requirement.implementation)
            .ok_or(MirErasedError::InvalidDictionary)?;
        if !used.contains(&(available.len() as u32))
            || template.parameters.is_empty()
            || requirement.arguments.len() != template.parameters.len()
            || template.parameters.iter().any(|p| p.kind != Kind::Type)
            || template
                .parameters
                .iter()
                .map(|p| p.source)
                .collect::<BTreeSet<_>>()
                .len()
                != template.parameters.len()
            || requirement.children.len() != template.bounds.len()
            || available.contains(&requirement.result)
            || template.result.substitute(&requirement.arguments, tc)? != requirement.result
        {
            return Err(MirErasedError::InvalidDictionary);
        }
        dictionary_template_key(contract, tc, template)?;
        for argument in &requirement.arguments {
            if argument.kind(&function.signature, contract, tc)? != Kind::Type {
                return Err(MirErasedError::KindMismatch);
            }
            match argument {
                MirErasedType::Concrete(id)
                    if !descriptor_identity::concrete_descriptor_is_sealed(contract, *id) =>
                {
                    return Err(MirErasedError::MissingLayout)
                }
                MirErasedType::Concrete(_)
                | MirErasedType::Parameter(_)
                | MirErasedType::Tuple(_)
                | MirErasedType::Nominal { .. } => {}
                _ => return Err(MirErasedError::UnsupportedOperation),
            }
            let mut scopes = BTreeSet::new();
            argument.scoped_binders(&mut scopes);
            if !scopes.is_empty() {
                return Err(MirErasedError::UnboundParameter);
            }
            if !function.signature.layouts.contains(argument)
                && !layouts.contains(argument)
                && !matches!(argument, MirErasedType::Concrete(id) if descriptor_identity::concrete_descriptor_is_sealed(contract, *id))
            {
                return Err(MirErasedError::MissingLayout);
            }
        }
        for (child, bound) in requirement.children.iter().zip(&template.bounds) {
            if available.get(*child as usize)
                != Some(&bound.substitute(&requirement.arguments, tc)?)
            {
                return Err(MirErasedError::InvalidDictionary);
            }
        }
        for (binding, member) in template.members.iter().zip(&template.result.members) {
            if !member.binder.parameters.is_empty() {
                super::validate_member_entry(&binding.entry, contract, tc)?;
                let authority = &contract.erased.member_entries[&binding.entry];
                if authority.implementation != template.implementation
                    || authority.captures != template.parameters
                    || authority.member != *member
                {
                    return Err(MirErasedError::InvalidDictionary);
                }
                continue;
            }
            let entry = contract
                .erased
                .functions
                .get(&binding.entry)
                .ok_or(MirErasedError::MissingCallable)?;
            let mut params = member.params.clone();
            if let Some(mode) = member.receiver.filter(|mode| *mode != ReceiverMode::Move) {
                params[0] = MirErasedType::Reference {
                    inner: Box::new(params[0].clone()),
                    mutable: mode == ReceiverMode::Mut,
                };
            }
            let mut expected_layouts = (0..template.parameters.len())
                .map(|index| MirErasedType::Parameter(index as u32))
                .collect::<Vec<_>>();
            for ty in params.iter().chain(std::iter::once(&member.ret)) {
                ty.collect_layouts(&mut expected_layouts, false);
            }
            if binding.entry.definition != binding.method_id || !binding.entry.static_args.is_empty()
                || entry.signature.parameters != template.parameters || entry.signature.dictionaries != template.bounds
                || entry.signature.params != params || entry.signature.ret != member.ret || entry.body.is_none()
                || entry.signature.require_borrow_free != erased_callback_storage_requirements(&entry.signature.params, &template.bounds)
                || entry.signature.layouts != expected_layouts
                || entry.signature.layouts.iter().any(|ty| !matches!(ty, MirErasedType::Parameter(_) | MirErasedType::Concrete(_))
                    && !matches!(ty, MirErasedType::Reference { inner, .. } if matches!(inner.as_ref(), MirErasedType::Parameter(_))))
                || entry.body.as_ref().is_some_and(|body| body.blocks.iter().flat_map(|b| &b.statements).any(|s|
                    matches!(s.value, MirErasedRvalue::NativeCall { .. })
                        || member.receiver.is_none() && matches!(s.value, MirErasedRvalue::NativeApply { .. }))) {
                return Err(MirErasedError::InvalidDictionary);
            }
        }
        available.push(requirement.result.clone());
    }
    Ok(available)
}

#[cfg(test)]
#[path = "dictionary_factory_tests.rs"]
pub(crate) mod tests;
