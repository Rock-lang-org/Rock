use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

/// Node identity is independent of source position, spelling, and provenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InvocationId(pub NodeId);

impl InvocationId {
    pub fn fresh() -> Result<Self, &'static str> {
        NodeId::fresh().map(Self)
    }
}

impl NodeId {
    pub fn fresh() -> Result<Self, &'static str> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        NEXT.try_update(Ordering::Relaxed, Ordering::Relaxed, |next| next.checked_add(1))
            .map(Self).map_err(|_| "AST node identity limit exceeded")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MacroOutputCategory {
    Items,
    Expression,
}
