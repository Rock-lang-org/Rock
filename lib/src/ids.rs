use std::marker::PhantomData;

use serde::{Deserialize, Serialize};

pub trait Idx: Copy + Eq {
    fn from_raw(raw: u32) -> Self;
    fn raw(self) -> u32;

    fn index(self) -> usize {
        self.raw() as usize
    }
}

macro_rules! define_id {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        pub struct $name(pub u32);

        impl Idx for $name {
            fn from_raw(raw: u32) -> Self {
                Self(raw)
            }

            fn raw(self) -> u32 {
                self.0
            }
        }
    };
}

define_id!(CrateId);
define_id!(ModuleId);
define_id!(LocalDefId);
define_id!(FunctionId);
define_id!(StructId);
define_id!(EnumId);
define_id!(TraitId);
define_id!(ImplId);
define_id!(FieldId);
define_id!(VariantId);
define_id!(AssocTypeId);
define_id!(MethodId);
define_id!(TypeId);
define_id!(TypeVarId);
define_id!(InstanceId);
define_id!(HirLocalId);
define_id!(LoanId);
define_id!(MovePathId);
define_id!(PlacePathId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DefId {
    pub crate_id: CrateId,
    pub local: LocalDefId,
}

impl DefId {
    pub fn new(crate_id: CrateId, local: LocalDefId) -> Self {
        Self { crate_id, local }
    }
}

#[derive(Debug)]
/// Allocates typed IDs monotonically.
///
/// The generator is intentionally not `Clone`; duplicating its counter would let
/// two generators mint the same IDs.
///
/// ```text
/// use rock_lib::ids::{IdGen, ModuleId};
///
/// let gen = IdGen::<ModuleId>::new();
/// let _copy = gen.clone();
/// ```
pub struct IdGen<I> {
    next: u32,
    _marker: PhantomData<fn() -> I>,
}

impl<I> Default for IdGen<I> {
    fn default() -> Self {
        Self {
            next: 0,
            _marker: PhantomData,
        }
    }
}

impl<I: Idx> IdGen<I> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_next_raw(next: u32) -> Self {
        Self {
            next,
            _marker: PhantomData,
        }
    }

    pub fn fresh(&mut self) -> I {
        let raw = self.next;
        self.next = self
            .next
            .checked_add(1)
            .expect("compiler ID generator exhausted u32 ID space");
        I::from_raw(raw)
    }

    pub fn next_raw(&self) -> u32 {
        self.next
    }
}

#[cfg(test)]
mod tests;
