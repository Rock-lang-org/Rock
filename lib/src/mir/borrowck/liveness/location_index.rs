use crate::mir::MirFunction;

use crate::ids::LoanId;
use crate::mir::dataflow::analyses::LoanTable;
use crate::mir::dataflow::BitSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoanLocationIndex {
    block_offsets: Vec<usize>,
    slots: Vec<BitSet<LoanId>>,
}

impl LoanLocationIndex {
    pub fn for_function(table: &LoanTable, func: &MirFunction) -> Self {
        let statement_counts: Vec<_> = func
            .basic_blocks
            .iter()
            .map(|block| block.statements.len() + 1)
            .collect();
        Self::from_table(table, func.basic_blocks.len(), &statement_counts)
    }

    pub(super) fn from_table(
        table: &LoanTable,
        block_count: usize,
        statement_counts: &[usize],
    ) -> Self {
        let mut block_offsets = Vec::with_capacity(block_count + 1);
        let mut next_offset = 0;
        for block_idx in 0..block_count {
            block_offsets.push(next_offset);
            next_offset += statement_counts.get(block_idx).copied().unwrap_or(0);
        }
        block_offsets.push(next_offset);

        let slots = (0..next_offset)
            .map(|_| BitSet::with_capacity(table.len()))
            .collect::<Vec<_>>();
        let mut index = Self {
            block_offsets,
            slots,
        };

        for loan in table.iter() {
            if let Some(slot) = index.slot_mut(loan.created_at) {
                slot.insert(loan.id);
            }
        }

        index
    }

    pub fn loan_ids_at(
        &self,
        location: crate::mir::borrowck::location::Location,
    ) -> impl Iterator<Item = LoanId> + '_ {
        self.slot(location).into_iter().flat_map(|slot| slot.iter())
    }

    fn slot(&self, location: crate::mir::borrowck::location::Location) -> Option<&BitSet<LoanId>> {
        let index = self.slot_index(location)?;
        self.slots.get(index)
    }

    fn slot_mut(
        &mut self,
        location: crate::mir::borrowck::location::Location,
    ) -> Option<&mut BitSet<LoanId>> {
        let index = self.slot_index(location)?;
        self.slots.get_mut(index)
    }

    fn slot_index(&self, location: crate::mir::borrowck::location::Location) -> Option<usize> {
        let block = location.block.0;
        let start = *self.block_offsets.get(block)?;
        let end = *self.block_offsets.get(block + 1)?;
        let index = start + location.statement.0;
        (index < end).then_some(index)
    }
}
