//! Member-local binders never alias parent dictionary captures or body scopes.
use std::collections::BTreeSet;

use crate::hir::borrowing::{BorrowContract, BorrowReturn, BorrowSource, BorrowSourceKind};
use crate::ids::DefId;
use crate::type_context::TypeContext;
use crate::type_services::kind::Kind;
use crate::types::ReceiverMode;

use super::*;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MirErasedMemberBinder<T = crate::ids::TypeId> {
    pub parameters: Vec<MirErasedParameter>,
    /// Explicitly unsupported in this first member-local increment, not omitted.
    pub dictionaries: Vec<MirErasedDictionarySchema<T>>,
}

impl<T> Default for MirErasedMemberBinder<T> {
    fn default() -> Self { Self { parameters: Vec::new(), dictionaries: Vec::new() } }
}

impl<T> MirErasedMemberBinder<T> {
    pub fn map_types<U: Ord, E>(&self, map: &mut impl FnMut(&T) -> Result<U, E>) -> Result<MirErasedMemberBinder<U>, E> {
        let dictionaries = self.dictionaries.iter().map(|schema| {
            Ok(MirErasedDictionarySchema {
                subject: schema.subject.map_types(map)?, trait_id: schema.trait_id,
                trait_args: schema.trait_args.iter().map(|ty| ty.map_types(map)).collect::<Result<_, E>>()?,
                members: schema.members.iter().map(|member| Ok(MirErasedDictionaryMember {
                    member_id: member.member_id, receiver: member.receiver, binder: member.binder.map_types(map)?,
                    params: member.params.iter().map(|ty| ty.map_types(map)).collect::<Result<_, E>>()?,
                    ret: member.ret.map_types(map)?, borrowing: member.borrowing.clone(),
                })).collect::<Result<_, E>>()?,
            })
        }).collect::<Result<_, E>>()?;
        Ok(MirErasedMemberBinder { parameters: self.parameters.clone(), dictionaries })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirDictionaryMemberTarget {
    Native(MirCallableKey),
    Polymorphic(MirErasedKey),
}

impl MirDictionaryMemberTarget {
    pub fn native(&self) -> Option<&MirCallableKey> {
        match self { Self::Native(key) => Some(key), Self::Polymorphic(_) => None }
    }
}

/// Checked again against accepted impl/method authority at the MIR boundary.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MirErasedMemberEntry<T = crate::ids::TypeId> {
    pub implementation: DefId,
    pub trait_id: DefId,
    pub method_id: DefId,
    pub captures: Vec<MirErasedParameter>,
    pub member: MirErasedDictionaryMember<T>,
}

impl<T> MirErasedMemberEntry<T> {
    pub fn map_types<U: Ord, E>(&self, map: &mut impl FnMut(&T) -> Result<U, E>) -> Result<MirErasedMemberEntry<U>, E> {
        Ok(MirErasedMemberEntry { implementation: self.implementation, trait_id: self.trait_id, method_id: self.method_id,
            captures: self.captures.clone(), member: MirErasedDictionaryMember {
                member_id: self.member.member_id, receiver: self.member.receiver, binder: self.member.binder.map_types(map)?,
                params: self.member.params.iter().map(|ty| ty.map_types(map)).collect::<Result<_, E>>()?,
                ret: self.member.ret.map_types(map)?, borrowing: self.member.borrowing.clone(),
            } })
    }
}

impl MirErasedMemberEntry<crate::types::Type> {
    pub fn try_remap_def_ids(&mut self, remap: &mut impl FnMut(DefId) -> Result<DefId, String>) -> Result<(), String> {
        self.implementation = remap(self.implementation)?;
        self.trait_id = remap(self.trait_id)?;
        self.method_id = remap(self.method_id)?;
        self.member.member_id = remap(self.member.member_id)?;
        for parameter in self.captures.iter_mut().chain(&mut self.member.binder.parameters) { parameter.source.owner = remap(parameter.source.owner)?; }
        if !self.member.binder.dictionaries.is_empty() { return Err("portable member-bound dictionary authority is unsupported".into()); }
        for ty in self.member.params.iter_mut().chain(std::iter::once(&mut self.member.ret)) {
            *ty = ty.map_types(&mut |ty| { let mut ty = ty.clone(); ty.try_remap_def_ids(remap)?; Ok::<_, String>(ty) })?;
            ty.try_remap_nominals(remap)?;
        }
        Ok(())
    }
}

impl<T: Clone> MirErasedType<T> {
    pub fn bind_member(&self, arguments: &[Self]) -> Result<Self, MirErasedError> {
        Ok(match self {
            Self::MemberParameter(index) => arguments.get(*index as usize).cloned().ok_or(MirErasedError::UnboundParameter)?,
            Self::Reference { inner, mutable } => Self::Reference { inner: Box::new(inner.bind_member(arguments)?), mutable: *mutable },
            Self::Pointer(inner) => Self::Pointer(Box::new(inner.bind_member(arguments)?)),
            Self::Tuple(fields) => Self::Tuple(fields.iter().map(|ty| ty.bind_member(arguments)).collect::<Result<_, _>>()?),
            Self::Array { element, len } => Self::Array { element: Box::new(element.bind_member(arguments)?), len: *len },
            Self::Nominal { id, flavor, args } => Self::Nominal { id: *id, flavor: *flavor, args: args.iter().map(|ty| ty.bind_member(arguments)).collect::<Result<_, _>>()? },
            Self::Apply { constructor, args } => Self::Apply { constructor: Box::new(constructor.bind_member(arguments)?), args: args.iter().map(|ty| ty.bind_member(arguments)).collect::<Result<_, _>>()? },
            _ => self.clone(),
        })
    }
}

pub(crate) fn member_binder_is_valid<T>(binder: &MirErasedMemberBinder<T>) -> bool {
    binder.dictionaries.is_empty()
        && binder.parameters.iter().all(|parameter| parameter.kind == Kind::Type)
        && binder.parameters.iter().map(|parameter| parameter.source).collect::<BTreeSet<_>>().len() == binder.parameters.len()
}

pub(crate) fn polymorphic_impl_member_bounds_are_captured<P: crate::hir::HirPhase>(
    implementation: &crate::hir::HirImplFor<P>, method: &crate::hir::HirFunctionFor<P>,
) -> bool {
    method.generic_bounds.iter().all(|(parameter, roots)| {
        implementation.type_generics.iter().any(|parent| parent.id == *parameter)
            && implementation.bounds.get(parameter).is_some_and(|captured| roots.iter().all(|root| captured.contains(root)))
    }) && method.generic_bounds.predicates.iter().all(|predicate| {
        let crate::types::Predicate::Trait { subject: crate::types::Type::Generic(parameter), trait_id, args } = predicate else { return false; };
        implementation.type_generics.iter().any(|parent| parent.id == *parameter)
            && implementation.bounds.get(parameter).is_some_and(|captured| captured.contains(&crate::types::TraitBound { trait_id: *trait_id, type_args: args.clone() }))
    })
}

pub(crate) fn validate_dictionary_binder_scope<T>(ty: &MirErasedType<T>, parents: usize, locals: usize) -> Result<(), MirErasedError> {
    match ty {
        MirErasedType::Parameter(index) if *index as usize >= parents => Err(MirErasedError::UnboundParameter),
        MirErasedType::MemberParameter(index) if *index as usize >= locals => Err(MirErasedError::UnboundParameter),
        MirErasedType::Scoped(_) => Err(MirErasedError::UnboundParameter),
        MirErasedType::Reference { inner, .. } | MirErasedType::Pointer(inner) => validate_dictionary_binder_scope(inner, parents, locals),
        MirErasedType::Tuple(fields) | MirErasedType::Nominal { args: fields, .. } => {
            for field in fields { validate_dictionary_binder_scope(field, parents, locals)?; } Ok(())
        }
        MirErasedType::Array { element, .. } => validate_dictionary_binder_scope(element, parents, locals),
        MirErasedType::Apply { constructor, args } => {
            validate_dictionary_binder_scope(constructor, parents, locals)?;
            for argument in args { validate_dictionary_binder_scope(argument, parents, locals)?; } Ok(())
        }
        _ => Ok(()),
    }
}

pub(crate) fn member_type_key(
    ty: &MirErasedType, locals: usize,
    parent: &impl Fn(&MirErasedType) -> Result<Vec<u8>, MirErasedError>,
) -> Result<Vec<u8>, MirErasedError> {
    let (tag, payload) = match ty {
        MirErasedType::MemberParameter(index) if (*index as usize) < locals => (b'm', (*index as u64).to_le_bytes().to_vec()),
        MirErasedType::MemberParameter(_) => return Err(MirErasedError::UnboundParameter),
        // Current bounded parametric signature admits only local values and a
        // parent receiver; other constructors need additional borrowing proof.
        _ => (b'c', parent(ty)?),
    };
    Ok(descriptor_identity::key(b"PM01", tag, &payload, &[]))
}

/// Universal local value variables cannot name the receiver/capture storage.
/// This first scope supports whole-value forwarding, not opaque retention or a
/// declared reference result; genuine bodies additionally prove these origins.
pub(crate) fn parametric_member_borrow_contract<T: PartialEq>(
    binder: &MirErasedMemberBinder<T>, params: &[MirErasedType<T>], ret: &MirErasedType<T>, receiver: Option<ReceiverMode>,
) -> Result<BorrowContract, MirErasedError> {
    if !member_binder_is_valid(binder) || binder.parameters.is_empty() || receiver != Some(ReceiverMode::Shared) || params.len() < 2 {
        return Err(MirErasedError::UnsupportedOperation);
    }
    let MirErasedType::MemberParameter(result) = ret else { return Err(MirErasedError::UnsupportedOperation); };
    if *result as usize >= binder.parameters.len() { return Err(MirErasedError::UnboundParameter); }
    let mut sources = Vec::new();
    for (index, ty) in params.iter().enumerate().skip(1) {
        let MirErasedType::MemberParameter(local) = ty else { return Err(MirErasedError::UnsupportedOperation); };
        if *local as usize >= binder.parameters.len() { return Err(MirErasedError::UnboundParameter); }
        if ty == ret { sources.push(BorrowSource { argument: index as u32, path: Vec::new(), kind: BorrowSourceKind::Value }); }
    }
    if sources.is_empty() { return Err(MirErasedError::InvalidDictionary); }
    Ok(BorrowContract { returns: vec![BorrowReturn { path: Vec::new(), sources }], retentions: Vec::new() })
}

pub(crate) fn dictionary_member_borrow_contract(
    binder: &MirErasedMemberBinder, params: &[MirErasedType], ret: &MirErasedType, receiver: Option<ReceiverMode>, tc: &TypeContext,
) -> Result<BorrowContract, MirErasedError> {
    if !member_binder_is_valid(binder) { return Err(MirErasedError::InvalidDictionary); }
    if binder.parameters.is_empty() { erased_borrow_contract(params, ret, receiver, tc) }
    else { parametric_member_borrow_contract(binder, params, ret, receiver) }
}

impl MirErasedDictionaryMember {
    /// Local descriptors belong to each member call, not the enclosing entry.
    pub(crate) fn collect_parent_layouts(&self, layouts: &mut Vec<MirErasedType>, witness: bool) {
        for ty in self.params.iter().chain(std::iter::once(&self.ret)) {
            // The admitted local shape is a whole value variable; constructed
            // local shapes remain subject to the strict member validator.
            if !matches!(ty, MirErasedType::MemberParameter(_)) {
                ty.collect_layouts(layouts, witness);
            }
        }
    }

    pub(crate) fn opened_types(&self, arguments: &[MirErasedType]) -> Result<(Vec<MirErasedType>, MirErasedType), MirErasedError> {
        if !member_binder_is_valid(&self.binder) || arguments.len() != self.binder.parameters.len() { return Err(MirErasedError::InvalidDictionary); }
        Ok((self.params.iter().map(|ty| ty.bind_member(arguments)).collect::<Result<_, _>>()?, self.ret.bind_member(arguments)?))
    }

    pub(crate) fn entry_signature(&self, captures: &[MirErasedParameter]) -> Result<MirErasedSignature, MirErasedError> {
        if (self.receiver.is_some() && self.params.is_empty()) || !member_binder_is_valid(&self.binder)
            || self.binder.parameters.iter().any(|local| captures.iter().any(|capture| capture.source == local.source)) {
            return Err(MirErasedError::InvalidDictionary);
        }
        for ty in self.params.iter().chain(std::iter::once(&self.ret)) {
            validate_dictionary_binder_scope(ty, captures.len(), self.binder.parameters.len())?;
        }
        let arguments = (0..self.binder.parameters.len()).map(|index| {
            u32::try_from(captures.len() + index).map(MirErasedType::Parameter).map_err(|_| MirErasedError::UnboundParameter)
        }).collect::<Result<Vec<_>, _>>()?;
        let (mut params, ret) = self.opened_types(&arguments)?;
        if let Some(receiver) = self.receiver.filter(|receiver| *receiver != ReceiverMode::Move) {
            params[0] = MirErasedType::Reference { inner: Box::new(params[0].clone()), mutable: receiver == ReceiverMode::Mut };
        }
        let parameters = captures.iter().chain(&self.binder.parameters).cloned().collect::<Vec<_>>();
        let mut layouts = (0..parameters.len()).map(|index| MirErasedType::Parameter(index as u32)).collect::<Vec<_>>();
        for ty in params.iter().chain(std::iter::once(&ret)) { ty.collect_layouts(&mut layouts, false); }
        Ok(MirErasedSignature { parameters, params, ret, dictionaries: Vec::new(), layouts, require_borrow_free: BTreeSet::new() })
    }
}

pub(crate) fn validate_member_entry(key: &MirErasedKey, contract: &MirBackendContract, tc: &TypeContext) -> Result<(), MirErasedError> {
    let authority = contract.erased.member_entries.get(key).ok_or(MirErasedError::InvalidDictionary)?;
    if contract.dictionary_member_authorities.get(key) != Some(authority) { return Err(MirErasedError::InvalidDictionary); }
    let implementation = contract.dictionary_impl_members.get(&authority.implementation).ok_or(MirErasedError::InvalidDictionary)?;
    if !implementation.factory_compatible || implementation.trait_id != authority.trait_id
        || implementation.members.get(&authority.member.member_id) != Some(&authority.method_id)
        || key.definition != authority.method_id || authority.member.binder.parameters.is_empty()
        || authority.member.borrowing != dictionary_member_borrow_contract(&authority.member.binder, &authority.member.params, &authority.member.ret, authority.member.receiver, tc)? {
        return Err(MirErasedError::InvalidDictionary);
    }
    let entry = contract.erased.functions.get(key).ok_or(MirErasedError::MissingCallable)?;
    let mut expected = authority.member.entry_signature(&authority.captures)?;
    if let Some(template) = &implementation.template { expected.dictionaries = template.bounds.clone(); }
    if entry.signature.parameters != expected.parameters || entry.signature.layouts != expected.layouts
        || entry.signature.params != expected.params || entry.signature.ret != expected.ret
        || entry.signature.dictionaries != expected.dictionaries || !entry.signature.require_borrow_free.is_empty()
        || entry.body.as_ref().is_some_and(|body| body.blocks.iter().flat_map(|block| &block.statements)
            .any(|statement| matches!(statement.value, MirErasedRvalue::NativeCall { .. }))) {
        return Err(MirErasedError::InvalidDictionary);
    }
    if entry.body.is_none() && (entry.linkage != MirLinkage::External
        || contract.erased.imported_member_links.get(key) != Some(&(entry.symbol.clone(), authority.clone()))) {
        return Err(MirErasedError::MissingCallable);
    }
    Ok(())
}

#[cfg(test)]
#[path = "polymorphic_dictionary_tests.rs"]
pub(crate) mod tests;
