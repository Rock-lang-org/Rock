use crate::mir::BasicBlockId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StatementIndex(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Location {
    pub block: BasicBlockId,
    pub statement: StatementIndex,
}

impl Location {
    pub fn new(block: BasicBlockId, statement: StatementIndex) -> Self {
        Self { block, statement }
    }
}

#[cfg(test)]
mod tests;
