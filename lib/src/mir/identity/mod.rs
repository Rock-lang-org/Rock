mod aggregate;
mod intrinsic;
mod intrinsic_names;

use crate::ids::{DefId, InstanceId, TypeId};

use super::backend_contract::MirCallableKey;

pub use aggregate::{MirAggregateIdentity, MirFieldIdentity};
pub use intrinsic::MirIntrinsicId;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MirFunctionId {
    Function(DefId),
    Extern(DefId),
    Instance(InstanceId),
    Closure(Box<MirClosureId>),
    ArrayTailDrop(TypeId),
}

impl MirFunctionId {
    pub fn display_fallback(self, _fallback: &str) -> String {
        match self {
            MirFunctionId::Function(id) => format!("def#{}:{}", id.crate_id.0, id.local.0),
            MirFunctionId::Extern(id) => format!("extern#{}:{}", id.crate_id.0, id.local.0),
            MirFunctionId::Instance(id) => format!("instance#{}", id.0),
            MirFunctionId::Closure(id) => format!("closure#{:?}:{}", id.owner, id.local_index),
            MirFunctionId::ArrayTailDrop(ty) => format!("array_tail_drop#{:?}", ty),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MirClosureId {
    pub owner: MirFunctionId,
    pub local_index: u32,
}

impl MirClosureId {
    /// Id of the synthesized function that drops this closure's environment.
    pub fn env_drop_id(&self) -> MirClosureId {
        MirClosureId {
            owner: MirFunctionId::Closure(Box::new(self.clone())),
            local_index: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MirRuntimeHelper {
    BoundsCheck,
    HeapAlloc,
    DropGlue,
    ArrayTailDrop(TypeId),
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
                MirCallableKey::RuntimeHelper(MirRuntimeHelper::ArrayTailDrop(ty)) => {
                    Some(MirFunctionId::ArrayTailDrop(*ty))
                }
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
