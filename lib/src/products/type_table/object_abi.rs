use super::{def_id, product_def_id, ProductTypeDecoder, ProductTypeEncoder, ProductTypeId};
use crate::mir::{MirObjectResult, MirPassMode};
use crate::products::object_abi::{
    ProductErasedObjectSlot, ProductObjectAbi, ProductObjectParam, ProductObjectSchema,
    ProductObjectSlot,
};
use crate::products::ProductDefId;
use crate::types::ReceiverMode;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SerializedObjectAbi {
    pub version: u32,
    pub target: Option<String>,
    pub trait_members: BTreeMap<ProductDefId, Vec<ProductDefId>>,
    pub schemas: Vec<SerializedObjectSchema>,
}

impl Default for SerializedObjectAbi {
    fn default() -> Self {
        Self {
            version: crate::products::object_abi::OBJECT_ABI_VERSION,
            target: None,
            trait_members: BTreeMap::new(),
            schemas: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SerializedObjectSchema {
    pub object: ProductTypeId,
    pub slots: Vec<SerializedObjectSlot>,
    pub erased_slots: Vec<SerializedErasedObjectSlot>,
    pub views: Vec<ProductTypeId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SerializedErasedObjectSlot {
    pub trait_id: ProductDefId,
    pub member_id: ProductDefId,
    pub trait_args: Vec<ProductTypeId>,
    pub receiver: ReceiverMode,
    pub signature: crate::mir::MirErasedSignature<ProductTypeId>,
    pub result: MirObjectResult,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SerializedObjectSlot {
    pub trait_id: ProductDefId,
    pub member_id: ProductDefId,
    pub trait_args: Vec<ProductTypeId>,
    pub receiver: ReceiverMode,
    pub params: Vec<(ProductTypeId, MirPassMode)>,
    pub ret: ProductTypeId,
    pub abi_ret: ProductTypeId,
    pub result: MirObjectResult,
}

impl SerializedObjectAbi {
    pub(super) fn encode(
        abi: &ProductObjectAbi,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        abi.validate_shape()?;
        Ok(Self {
            version: abi.version,
            target: abi.target.clone(),
            trait_members: abi
                .trait_members
                .iter()
                .map(|(id, members)| {
                    (
                        product_def_id(*id),
                        members.iter().copied().map(product_def_id).collect(),
                    )
                })
                .collect(),
            schemas: abi
                .schemas
                .iter()
                .map(|schema| {
                    Ok(SerializedObjectSchema {
                        object: encoder.encode_type(&schema.object)?,
                        erased_slots: schema
                            .erased_slots
                            .iter()
                            .map(|slot| {
                                Ok(SerializedErasedObjectSlot {
                                    trait_id: product_def_id(slot.trait_id),
                                    member_id: product_def_id(slot.member_id),
                                    trait_args: encoder.encode_types(&slot.trait_args)?,
                                    receiver: slot.receiver,
                                    signature: slot
                                        .signature
                                        .map_types(&mut |ty| encoder.encode_type(ty))?,
                                    result: slot.result,
                                })
                            })
                            .collect::<Result<_, String>>()?,
                        views: encoder.encode_types(&schema.views)?,
                        slots: schema
                            .slots
                            .iter()
                            .map(|slot| {
                                Ok(SerializedObjectSlot {
                                    trait_id: product_def_id(slot.trait_id),
                                    member_id: product_def_id(slot.member_id),
                                    trait_args: encoder.encode_types(&slot.trait_args)?,
                                    receiver: slot.receiver,
                                    params: slot
                                        .params
                                        .iter()
                                        .map(|param| {
                                            encoder
                                                .encode_type(&param.ty)
                                                .map(|ty| (ty, param.pass_mode))
                                        })
                                        .collect::<Result<_, _>>()?,
                                    ret: encoder.encode_type(&slot.ret)?,
                                    abi_ret: encoder.encode_type(&slot.abi_ret)?,
                                    result: slot.result,
                                })
                            })
                            .collect::<Result<_, String>>()?,
                    })
                })
                .collect::<Result<_, String>>()?,
        })
    }

    pub(super) fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<ProductObjectAbi, String> {
        let result = ProductObjectAbi {
            version: self.version,
            target: self.target,
            trait_members: self
                .trait_members
                .into_iter()
                .map(|(id, members)| (def_id(id), members.into_iter().map(def_id).collect()))
                .collect(),
            schemas: self
                .schemas
                .into_iter()
                .map(|schema| {
                    Ok(ProductObjectSchema {
                        object: decoder.decode_type(schema.object)?,
                        erased_slots: schema
                            .erased_slots
                            .into_iter()
                            .map(|slot| {
                                Ok(ProductErasedObjectSlot {
                                    trait_id: def_id(slot.trait_id),
                                    member_id: def_id(slot.member_id),
                                    trait_args: slot
                                        .trait_args
                                        .into_iter()
                                        .map(|id| decoder.decode_object_component(id))
                                        .collect::<Result<_, _>>()?,
                                    receiver: slot.receiver,
                                    signature: slot
                                        .signature
                                        .map_types(&mut |id| decoder.decode_type(*id))?,
                                    result: slot.result,
                                })
                            })
                            .collect::<Result<_, String>>()?,
                        views: decoder.decode_types(&schema.views)?,
                        slots: schema
                            .slots
                            .into_iter()
                            .map(|slot| {
                                Ok(ProductObjectSlot {
                                    trait_id: def_id(slot.trait_id),
                                    member_id: def_id(slot.member_id),
                                    trait_args: slot
                                        .trait_args
                                        .into_iter()
                                        .map(|id| decoder.decode_object_component(id))
                                        .collect::<Result<_, _>>()?,
                                    receiver: slot.receiver,
                                    params: slot
                                        .params
                                        .into_iter()
                                        .map(|(ty, pass_mode)| {
                                            decoder
                                                .decode_type(ty)
                                                .map(|ty| ProductObjectParam { ty, pass_mode })
                                        })
                                        .collect::<Result<_, _>>()?,
                                    ret: decoder.decode_type(slot.ret)?,
                                    abi_ret: decoder.decode_type(slot.abi_ret)?,
                                    result: slot.result,
                                })
                            })
                            .collect::<Result<_, String>>()?,
                    })
                })
                .collect::<Result<_, String>>()?,
        };
        result.validate_shape()?;
        Ok(result)
    }
}

pub(super) fn validate_component_scope(
    ty: &crate::types::Type,
    object_depth: u32,
) -> Result<(), String> {
    use crate::type_services::visit::{visit_type, visit_type_children, TypeVisitor};
    use crate::types::Type;
    struct Scope {
        objects: u32,
        lambdas: Vec<Vec<crate::type_services::kind::Kind>>,
        invalid: Option<&'static str>,
    }
    impl TypeVisitor for Scope {
        fn enter_object(&mut self) {
            self.objects += 1;
        }
        fn exit_object(&mut self) {
            self.objects -= 1;
        }
        fn enter_binders(&mut self, kinds: &[crate::type_services::kind::Kind]) {
            self.lambdas.push(kinds.to_vec());
        }
        fn exit_binders(&mut self) {
            self.lambdas.pop();
        }
        fn visit_type(&mut self, ty: &Type) {
            match ty {
                Type::ObjectSelf { depth } if *depth >= self.objects => {
                    self.invalid = Some("out-of-scope product object Self component")
                }
                Type::BoundVar { depth, index, kind }
                    if self
                        .lambdas
                        .iter()
                        .rev()
                        .nth(*depth as usize)
                        .and_then(|kinds| kinds.get(*index as usize))
                        != Some(kind) =>
                {
                    self.invalid =
                        Some("out-of-scope or kind-mismatched product bound variable component")
                }
                _ => {}
            }
            visit_type_children(ty, self);
        }
    }
    let mut scope = Scope {
        objects: object_depth,
        lambdas: Vec::new(),
        invalid: None,
    };
    visit_type(ty, &mut scope);
    scope.invalid.map_or(Ok(()), |error| Err(error.into()))
}
