//! Exact runtime metadata keys, independent of session interner IDs and symbols.
//! RK01/RD01: magic, constructor byte, framed payload, child count, framed keys.
use std::collections::{BTreeMap, BTreeSet};

use crate::ids::{CrateId, DefId, TypeId};
use crate::products::ProductCrateIdentity;
use crate::type_context::{Ty, TypeContext};
use crate::types::Type;

use super::{MirBackendContract, MirPayloadDropChildren, MirPayloadDropPlan};

pub const DESCRIPTOR_ABI: u64 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirDescriptorIdentity {
    pub semantic: Vec<u8>,
    pub drop_template: Vec<u8>,
}

fn frame(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    out.extend_from_slice(bytes);
}

pub(crate) fn key(magic: &[u8; 4], tag: u8, payload: &[u8], children: &[Vec<u8>]) -> Vec<u8> {
    let mut out = magic.to_vec();
    out.push(tag);
    frame(&mut out, payload);
    out.extend_from_slice(&(children.len() as u64).to_le_bytes());
    for child in children {
        frame(&mut out, child);
    }
    out
}

pub fn tuple_identity(children: &[Vec<u8>]) -> Vec<u8> {
    key(b"RK01", b't', &[], children)
}

pub fn tuple_drop_template(children: &[Vec<u8>]) -> Vec<u8> {
    // MirPayloadDropChildren::Fields specifies reverse field destruction.
    key(b"RD01", b't', &[], children)
}

pub(crate) fn definition(
    id: DefId,
    origins: &BTreeMap<CrateId, ProductCrateIdentity>,
) -> Option<Vec<u8>> {
    let origin = origins.get(&id.crate_id)?;
    let mut out = Vec::new();
    // Same complete product-origin inputs as mono::write_crate_identity, unhashed.
    for text in [origin.name.as_str(), origin.version.as_str()] {
        frame(&mut out, text.as_bytes());
    }
    for text in [
        origin.target_triple.as_deref(),
        origin.source_fingerprint.manifest_hash.as_deref(),
        origin.source_fingerprint.source_hash.as_deref(),
    ] {
        out.push(u8::from(text.is_some()));
        if let Some(text) = text {
            frame(&mut out, text.as_bytes());
        }
    }
    out.extend_from_slice(&origin.format_version.to_le_bytes());
    out.extend_from_slice(&(origin.source_fingerprint.loaded_files.len() as u64).to_le_bytes());
    for path in &origin.source_fingerprint.loaded_files {
        frame(&mut out, path.to_str()?.as_bytes());
    }
    out.extend_from_slice(&id.local.0.to_le_bytes());
    Some(out)
}

pub fn semantic_identity(
    contract: &MirBackendContract,
    tc: &TypeContext,
    ty: TypeId,
) -> Option<Vec<u8>> {
    fn encode(
        contract: &MirBackendContract,
        tc: &TypeContext,
        id: TypeId,
        active: &mut BTreeSet<TypeId>,
    ) -> Option<Vec<u8>> {
        let id = contract.normalize_type_id(tc, id)?;
        if !active.insert(id) {
            return None;
        }
        let children = |ids: &[TypeId], active: &mut BTreeSet<TypeId>| {
            ids.iter()
                .map(|id| encode(contract, tc, *id, active))
                .collect::<Option<Vec<_>>>()
        };
        let result = match tc.try_ty(id)? {
            Ty::Tuple(fields) => tuple_identity(&children(fields, active)?),
            Ty::Struct { id, args } | Ty::StructRepresentation { id, args, .. } => key(
                b"RK01",
                b'S',
                &definition(*id, &contract.descriptor_origins)?,
                &children(args, active)?,
            ),
            Ty::Enum { id, args } | Ty::EnumRepresentation { id, args, .. } => key(
                b"RK01",
                b'E',
                &definition(*id, &contract.descriptor_origins)?,
                &children(args, active)?,
            ),
            Ty::Reference { inner, mutable } => key(
                b"RK01",
                if *mutable { b'R' } else { b'r' },
                &[],
                &children(&[*inner], active)?,
            ),
            Ty::Pointer(inner) => key(b"RK01", b'p', &[], &children(&[*inner], active)?),
            Ty::Slice(inner) => key(b"RK01", b's', &[], &children(&[*inner], active)?),
            Ty::Array { inner, len } => key(
                b"RK01",
                b'A',
                &(*len as u64).to_le_bytes(),
                &children(&[*inner], active)?,
            ),
            atom => {
                let name: &[u8] = match atom {
                    Ty::I8 => b"i8",
                    Ty::I16 => b"i16",
                    Ty::I32 => b"i32",
                    Ty::I64 => b"i64",
                    Ty::U8 => b"u8",
                    Ty::U16 => b"u16",
                    Ty::U32 => b"u32",
                    Ty::U64 => b"u64",
                    Ty::F32 => b"f32",
                    Ty::F64 => b"f64",
                    Ty::Bool => b"bool",
                    Ty::Char => b"char",
                    Ty::Str => b"str",
                    Ty::Unit => b"unit",
                    Ty::Never => b"never",
                    // lean-ctx: exact concrete leaves ceiling; binder/function/object keys
                    // require their own canonical runtime protocol, never a layout key.
                    _ => return None,
                };
                key(b"RK01", b'a', name, &[])
            }
        };
        active.remove(&id);
        Some(result)
    }
    encode(contract, tc, ty, &mut BTreeSet::new())
}

pub fn descriptor_identity(
    contract: &MirBackendContract,
    tc: &TypeContext,
    plan: &MirPayloadDropPlan,
) -> Option<MirDescriptorIdentity> {
    fn drop_key(
        contract: &MirBackendContract,
        tc: &TypeContext,
        plan: &MirPayloadDropPlan,
    ) -> Option<Vec<u8>> {
        let semantic = semantic_identity(contract, tc, plan.ty)?;
        if let (Some(Ty::Struct { id, .. }), Some(callable)) = (tc.try_ty(plan.ty), &plan.user_drop) {
            if let Some(super::MirNominalDropAuthority::Selected(selection)) = contract.nominal_drop_authority.get(id) {
                let declaration = contract.callables.get(callable)?;
                let unit = tc.id_for_type(&Type::Unit)?;
                if declaration.source_def_id != Some(selection.entry.definition)
                    || declaration.signature != super::MirCallableSignature::from_instance_type_ids(&[plan.ty], unit, true, tc)
                { return None; }
                let (template, templates) = super::nominal_template(contract, tc, *id).ok()?;
                let fields = match &plan.children {
                    MirPayloadDropChildren::Fields(fields) => fields.as_slice(),
                    MirPayloadDropChildren::None if templates.is_empty() => &[],
                    _ => return None,
                };
                if fields.len() != templates.len() { return None; }
                let mut payload = Vec::new();
                frame(&mut payload, &semantic);
                frame(&mut payload, &template);
                frame(&mut payload, &super::nominal_factory::nominal_drop_template_key(contract, tc, selection).ok()?);
                return Some(key(b"RD01", b'h', &payload, &fields.iter()
                    .map(|field| drop_key(contract, tc, field)).collect::<Option<Vec<_>>>()?));
            }
        }
        if plan.user_drop.is_none() {
            if let Some(Ty::Struct { id, .. }) = tc.try_ty(plan.ty) {
                if let Ok((template, templates)) = super::nominal_template(contract, tc, *id) {
                    let fields = match &plan.children {
                        MirPayloadDropChildren::Fields(fields) => Some(fields.as_slice()),
                        MirPayloadDropChildren::None if templates.is_empty() => Some(&[][..]),
                        _ => None,
                    };
                    if let Some(fields) = fields {
                        let mut payload = Vec::new();
                        frame(&mut payload, &semantic);
                        payload.push(0);
                        frame(&mut payload, &template);
                        return Some(key(
                            b"RD01",
                            b'g',
                            &payload,
                            &fields
                                .iter()
                                .map(|field| drop_key(contract, tc, field))
                                .collect::<Option<Vec<_>>>()?,
                        ));
                    }
                }
            }
            if matches!(tc.try_ty(plan.ty), Some(Ty::Tuple(fields)) if fields.is_empty())
                && matches!(plan.children, MirPayloadDropChildren::None)
            {
                return Some(tuple_drop_template(&[]));
            }
            if let (Some(Ty::Tuple(fields)), MirPayloadDropChildren::Fields(plans)) =
                (tc.try_ty(plan.ty), &plan.children)
            {
                if fields.len() != plans.len()
                    || !fields.iter().zip(plans).all(|(ty, plan)| *ty == plan.ty)
                {
                    return None;
                }
                return Some(tuple_drop_template(
                    &plans
                        .iter()
                        .map(|plan| drop_key(contract, tc, plan))
                        .collect::<Option<Vec<_>>>()?,
                ));
            }
        }
        let mut payload = Vec::new();
        frame(&mut payload, &semantic);
        if let Some(callable) = &plan.user_drop {
            let declaration = contract.callables.get(callable)?;
            payload.push(1);
            frame(
                &mut payload,
                &definition(declaration.source_def_id?, &contract.descriptor_origins)?,
            );
            payload.extend_from_slice(&(declaration.signature.params.len() as u64).to_le_bytes());
            for param in &declaration.signature.params {
                frame(
                    &mut payload,
                    &semantic_identity(contract, tc, param.semantic_ty)?,
                );
                payload.push(match param.pass_mode {
                    super::MirPassMode::Direct => 0,
                    super::MirPassMode::Pointer => 1,
                    super::MirPassMode::FatDirect => 2,
                });
            }
            frame(
                &mut payload,
                &semantic_identity(contract, tc, declaration.signature.ret.semantic_ty)?,
            );
            frame(
                &mut payload,
                &semantic_identity(contract, tc, declaration.signature.ret.abi_ty)?,
            );
        } else {
            payload.push(0);
        }
        let (tag, children) = match &plan.children {
            MirPayloadDropChildren::None => (b'n', Vec::new()),
            MirPayloadDropChildren::Fields(fields) => (
                b'f',
                fields
                    .iter()
                    .map(|p| drop_key(contract, tc, p))
                    .collect::<Option<Vec<_>>>()?,
            ),
            MirPayloadDropChildren::Array { len, element } => {
                payload.extend_from_slice(&(*len as u64).to_le_bytes());
                (b'a', vec![drop_key(contract, tc, element)?])
            }
            // Runtime package/closure/variant templates are outside this tuple foundation.
            _ => return None,
        };
        Some(key(b"RD01", tag, &payload, &children))
    }
    Some(MirDescriptorIdentity {
        semantic: semantic_identity(contract, tc, plan.ty)?,
        drop_template: drop_key(contract, tc, plan)?,
    })
}

pub fn validate_descriptor_identities(contract: &MirBackendContract, tc: &TypeContext) -> bool {
    contract.descriptor_identities.iter().all(|(ty, supplied)| {
        contract
            .erased
            .descriptors
            .get(ty)
            .and_then(|descriptor| descriptor_identity(contract, tc, &descriptor.drop))
            .as_ref()
            == Some(supplied)
    })
}

pub fn concrete_descriptor_is_sealed(contract: &MirBackendContract, ty: TypeId) -> bool {
    fn visit(contract: &MirBackendContract, ty: TypeId, active: &mut BTreeSet<TypeId>) -> bool {
        if !active.insert(ty) {
            return false;
        }
        let valid = contract.descriptor_identities.contains_key(&ty)
            && contract
                .erased
                .descriptors
                .get(&ty)
                .is_some_and(|descriptor| {
                    descriptor
                        .fields
                        .iter()
                        .all(|child| visit(contract, *child, active))
                });
        active.remove(&ty);
        valid
    }
    visit(contract, ty, &mut BTreeSet::new())
}

#[cfg(test)]
#[path = "descriptor_identity_tests.rs"]
mod tests;
