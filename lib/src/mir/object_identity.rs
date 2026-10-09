//! Portable member evidence keys; runtime lookup compares opaque bytes only.
//! Slot positions and the surrounding guarantee subset are not member identity.

use std::collections::BTreeMap;

use crate::ids::{CrateId, DefId, LocalDefId};
use crate::type_context::TypeContext;
use crate::types::Type;

use super::{MirBackendContract, MirObjectResult, MirObjectSchema};

fn portable<T: serde::Serialize>(
    value: &mut T,
    origins: &BTreeMap<CrateId, crate::products::ProductCrateIdentity>,
    mut remap: impl FnMut(&mut T, &mut dyn FnMut(DefId) -> Result<DefId, String>) -> Result<(), String>,
) -> Result<Vec<u8>, String> {
    let mut definitions = BTreeMap::new();
    remap(value, &mut |id| {
        let key = super::descriptor_identity::definition(id, origins)
            .ok_or("object member lacks its defining product identity")?;
        definitions.insert(key, 0u32);
        Ok(id)
    })?;
    let mut out = Vec::new();
    out.extend_from_slice(&(definitions.len() as u64).to_le_bytes());
    for (index, (key, local)) in definitions.iter_mut().enumerate() {
        *local =
            u32::try_from(index).map_err(|_| "object member identity exceeds metadata budget")?;
        super::dictionary_factory::dictionary_frame(&mut out, key);
    }
    remap(value, &mut |id| {
        let key = super::descriptor_identity::definition(id, origins)
            .ok_or("object member lacks its defining product identity")?;
        let local = definitions
            .get(&key)
            .ok_or("object member identity changed during remapping")?;
        Ok(DefId::new(CrateId(0), LocalDefId(*local)))
    })?;
    let shape = bincode::serialize(value).map_err(|error| error.to_string())?;
    super::dictionary_factory::dictionary_frame(&mut out, &shape);
    Ok(out)
}

pub(crate) fn member_key(
    contract: &MirBackendContract,
    tc: &TypeContext,
    schema: &MirObjectSchema,
    field: u32,
    index: usize,
) -> Result<Vec<u8>, String> {
    let (trait_id, member_id, arguments, receiver, result) = match field {
        3 => {
            let slot = schema
                .slots
                .get(index)
                .ok_or("object member slot missing")?;
            (
                slot.trait_id,
                slot.member_id,
                &slot.trait_args,
                slot.receiver,
                slot.result,
            )
        }
        6 => {
            let slot = schema
                .erased_slots
                .get(index)
                .ok_or("erased object member slot missing")?;
            (
                slot.trait_id,
                slot.member_id,
                &slot.trait_args,
                slot.receiver,
                slot.result,
            )
        }
        _ => return Err("object lookup is not a member evidence operation".into()),
    };
    let mut out = b"OM02".to_vec();
    out.push(field as u8);
    for id in [trait_id, member_id] {
        let identity = super::descriptor_identity::definition(id, &contract.descriptor_origins)
            .ok_or("object member lacks its defining product identity")?;
        super::dictionary_factory::dictionary_frame(&mut out, &identity);
    }
    out.push(match receiver {
        crate::types::ReceiverMode::Shared => 0,
        crate::types::ReceiverMode::Mut => 1,
        crate::types::ReceiverMode::Move => 2,
    });
    out.push(match result {
        MirObjectResult::Direct => 0,
        MirObjectResult::BorrowedSelf { mutable: false } => 1,
        MirObjectResult::BorrowedSelf { mutable: true } => 2,
    });
    let mut args = Type::Tuple(arguments.iter().map(|id| tc.type_for(*id)).collect());
    let encode_types = |value: &mut Type, remap: &mut dyn FnMut(DefId) -> Result<DefId, String>| {
        value.try_remap_def_ids(&mut |id| remap(id))
    };
    super::dictionary_factory::dictionary_frame(
        &mut out,
        &portable(&mut args, &contract.descriptor_origins, encode_types)?,
    );
    let shape = if field == 3 {
        let signature = &schema.slots[index].signature;
        out.extend_from_slice(&(signature.params.len().saturating_sub(1) as u64).to_le_bytes());
        for parameter in signature.params.iter().skip(1) {
            out.push(match parameter.pass_mode {
                super::MirPassMode::Direct => 0,
                super::MirPassMode::Pointer => 1,
                super::MirPassMode::FatDirect => 2,
            });
        }
        // Borrowed Self is repacked with the caller's complete evidence. Its
        // concrete entry ABI is a thin pointer, not a guarantee-subset handle.
        let ret = match result {
            MirObjectResult::Direct => tc.type_for(signature.ret.abi_ty),
            MirObjectResult::BorrowedSelf { .. } => Type::Unit,
        };
        let mut shape = Type::function(
            signature
                .params
                .iter()
                .skip(1)
                .map(|p| tc.type_for(p.semantic_ty))
                .collect(),
            ret,
        );
        portable(&mut shape, &contract.descriptor_origins, encode_types)?
    } else {
        let mut shape = schema.erased_slots[index].signature.map_types(&mut |id| {
            tc.type_id_tree_is_valid(*id)
                .then(|| tc.type_for(*id))
                .ok_or_else(|| "invalid object member type".to_string())
        })?;
        portable(
            &mut shape,
            &contract.descriptor_origins,
            |signature, remap| {
                *signature = signature.map_types(&mut |ty| {
                    let mut ty = ty.clone();
                    ty.try_remap_def_ids(&mut |id| remap(id))?;
                    Ok::<_, String>(ty)
                })?;
                crate::products::object_abi::remap_erased_signature_ids(signature, &mut |id| {
                    remap(id)
                })
            },
        )?
    };
    super::dictionary_factory::dictionary_frame(&mut out, &shape);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::products::object_abi::ProductObjectSchema;
    use crate::type_services::normalize::TypeNormalizationEnv;

    fn fixture() -> (
        MirBackendContract,
        TypeContext,
        MirObjectSchema,
        MirObjectSchema,
    ) {
        use crate::types::{ObjectType, TraitBound};
        let definition = |index| DefId::new(CrateId(0), LocalDefId(index));
        let bound = |index| TraitBound {
            trait_id: definition(index),
            type_args: vec![],
        };
        let mut tc = TypeContext::new();
        let scalar = tc.intern_type(&Type::I64);
        let mut object = ObjectType::new(bound(1));
        object.guarantees.insert(bound(2));
        let source = tc.intern_type(&Type::Object(Box::new(object)));
        let target = tc.intern_type(&Type::Object(Box::new(ObjectType::new(bound(2)))));
        let mut slot = |object, member| {
            let receiver = tc.intern_type(&Type::Reference {
                mutable: false,
                inner: Box::new(tc.type_for(object)),
            });
            let mut signature = super::super::MirCallableSignature::from_type_ids(
                &[receiver],
                scalar,
                super::super::MirPassMode::Direct,
            );
            signature.params[0].pass_mode = super::super::MirPassMode::FatDirect;
            super::super::MirObjectSlot {
                trait_id: definition(member),
                member_id: definition(member + 10),
                trait_args: vec![],
                receiver: crate::types::ReceiverMode::Shared,
                signature,
                result: MirObjectResult::Direct,
            }
        };
        let source_schema = MirObjectSchema {
            object: source,
            slots: vec![slot(source, 1), slot(source, 2)],
            erased_slots: vec![],
            views: vec![target],
        };
        let target_schema = MirObjectSchema {
            object: target,
            slots: vec![slot(target, 2)],
            erased_slots: vec![],
            views: vec![],
        };
        let mut contract = MirBackendContract::default();
        contract.descriptor_origins.insert(
            CrateId(0),
            crate::products::ProductCrateIdentity::local("member-fixture".into()),
        );
        (contract, tc, source_schema, target_schema)
    }

    #[test]
    fn member_identity_survives_view_slot_and_importer_renumbering() {
        let (original, original_tc, source, target) = fixture();
        let key = member_key(&original, &original_tc, &source, 3, 1).unwrap();
        assert_eq!(
            key,
            member_key(&original, &original_tc, &target, 3, 0).unwrap(),
            "a consumer-selected view must not require the producer's slot position"
        );
        let mut product = ProductObjectSchema::from_mir(&source, &original_tc).unwrap();
        product
            .try_remap_def_ids(&mut |id| Ok(DefId::new(CrateId(91), id.local)))
            .unwrap();
        let mut tc = TypeContext::new();
        tc.intern_type(&Type::F32);
        let imported = product
            .intern(&mut tc, &TypeNormalizationEnv::default())
            .unwrap();
        let mut contract = MirBackendContract::default();
        contract.descriptor_origins.insert(
            CrateId(91),
            original.descriptor_origins[&CrateId(0)].clone(),
        );
        assert_eq!(key, member_key(&contract, &tc, &imported, 3, 1).unwrap());
        contract.descriptor_origins.clear();
        assert!(
            member_key(&contract, &tc, &imported, 3, 1).is_err(),
            "an importer ID without a defining origin is not portable member evidence"
        );
    }

    #[test]
    fn member_identity_keeps_exact_protocol_arguments_receiver_and_result_abi() {
        let (contract, original_tc, source, _) = fixture();
        let key = member_key(&contract, &original_tc, &source, 3, 1).unwrap();
        for mutation in 0..4 {
            let mut schema = source.clone();
            let mut tc = original_tc.clone();
            match mutation {
                0 => schema.slots[1].member_id.local.0 += 1,
                1 => schema.slots[1].trait_args.push(tc.intern_type(&Type::I64)),
                2 => schema.slots[1].receiver = crate::types::ReceiverMode::Mut,
                3 => schema.slots[1].signature.ret.abi_ty = tc.intern_type(&Type::Bool),
                _ => unreachable!(),
            }
            assert_ne!(key, member_key(&contract, &tc, &schema, 3, 1).unwrap());
        }
    }
}
