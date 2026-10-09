//! Implementation-local tuple construction evidence; never public layout args.
use super::MirErasedType;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirTupleFactoryRequirement {
    pub descriptor_abi: u64,
    pub result: MirErasedType,
    pub children: Vec<MirErasedType>,
}

#[cfg(test)]
#[path = "tuple_factory_tests.rs"]
pub(crate) mod tests;
