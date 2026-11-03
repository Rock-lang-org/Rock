mod aggregate;
mod intrinsic;
mod intrinsic_names;

use crate::ids::{DefId, InstanceId};

use super::backend_contract::MirCallableKey;

pub use aggregate::{MirAggregateIdentity, MirFieldIdentity};
pub use intrinsic::MirIntrinsicId;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MirFunctionId {
    Function(DefId),
    Extern(DefId),
    Instance(InstanceId),
    Closure(Box<MirClosureId>),
}

impl MirFunctionId {
    pub fn display_fallback(self, _fallback: &str) -> String {
        match self {
            MirFunctionId::Function(id) => format!("def#{}:{}", id.crate_id.0, id.local.0),
            MirFunctionId::Extern(id) => format!("extern#{}:{}", id.crate_id.0, id.local.0),
            MirFunctionId::Instance(id) => format!("instance#{}", id.0),
            MirFunctionId::Closure(id) => format!("closure#{:?}:{}", id.owner, id.local_index),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MirClosureId {
    pub owner: MirFunctionId,
    pub local_index: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MirRuntimeHelper {
    BoundsCheck,
    HeapAlloc,
    DropGlue,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MirCallable {
    Resolved(MirCallableKey),
}

impl MirCallable {
    pub fn function_id(&self) -> Option<MirFunctionId> {
        match self {
            MirCallable::Resolved(key) => match key {
                MirCallableKey::Function(id) => Some(MirFunctionId::Function(*id)),
                MirCallableKey::Extern(id) => Some(MirFunctionId::Extern(*id)),
                MirCallableKey::Instance(id) => Some(MirFunctionId::Instance(*id)),
                MirCallableKey::Closure(id) => Some(id.clone()),
                MirCallableKey::Intrinsic(_) | MirCallableKey::RuntimeHelper(_) => None,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MirAssertKind {
    BoundsCheck,
}

#[cfg(test)]
mod tests;
