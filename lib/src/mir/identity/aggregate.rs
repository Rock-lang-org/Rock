use std::hash::{Hash, Hasher};

use crate::ids::{DefId, FieldId, VariantId};

#[derive(Debug, Clone)]
pub enum MirAggregateIdentity {
    Struct {
        id: DefId,
        display_name: String,
    },
    EnumVariant {
        enum_id: DefId,
        variant_id: VariantId,
        enum_name: String,
        variant_name: String,
    },
}

impl PartialEq for MirAggregateIdentity {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                MirAggregateIdentity::Struct { id: left, .. },
                MirAggregateIdentity::Struct { id: right, .. },
            ) => left == right,
            (
                MirAggregateIdentity::EnumVariant {
                    enum_id: left_enum_id,
                    variant_id: left_variant_id,
                    ..
                },
                MirAggregateIdentity::EnumVariant {
                    enum_id: right_enum_id,
                    variant_id: right_variant_id,
                    ..
                },
            ) => left_enum_id == right_enum_id && left_variant_id == right_variant_id,
            _ => false,
        }
    }
}

impl Eq for MirAggregateIdentity {}

impl Hash for MirAggregateIdentity {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            MirAggregateIdentity::Struct { id, .. } => {
                0_u8.hash(state);
                id.hash(state);
            }
            MirAggregateIdentity::EnumVariant {
                enum_id,
                variant_id,
                ..
            } => {
                1_u8.hash(state);
                enum_id.hash(state);
                variant_id.hash(state);
            }
        }
    }
}

impl MirAggregateIdentity {
    pub fn def_id(&self) -> DefId {
        match self {
            MirAggregateIdentity::Struct { id, .. } => *id,
            MirAggregateIdentity::EnumVariant { enum_id, .. } => *enum_id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MirFieldIdentity {
    pub owner: DefId,
    pub field_id: FieldId,
}

impl MirFieldIdentity {
    pub fn new(owner: DefId, field_id: FieldId) -> Self {
        Self { owner, field_id }
    }
}
